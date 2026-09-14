# [Codename TBD] / Personal CFO: Personal Finance Desktop App Project Plan

**Working codename:** [TBD — run trademark/namespace check before committing]  
**Product description:** A local-first personal CFO for households  
**Platform target:** macOS desktop first, with a codebase structured for future Linux and Windows support  
**Distribution target:** Private GitHub repository first; public open-source release only after security hardening, core feature stabilization, and external review  
**Document version:** 2.2 proposed revision, scope-gated hybrid persistence/reliability review  
**Last updated:** 2026-05-02  
**Source:** User-provided project concept and requirements

---

## 0. Executive Decision Summary

The strongest version of this project is not a generic budget tracker. It is a **local-first household finance operating system**: encrypted, private, auditable, forecast-oriented, and useful even without automated bank connections.

The product should answer questions most consumer finance tools answer poorly:

> **How much cash will this household probably have in the future, what assumptions drive that forecast, and what risks are emerging before they become painful?**

This plan makes seven major strategic decisions:

1. **Use Tauri v2 + Rust + React/TypeScript as the primary desktop architecture.**  
   For this open-source project, the best practical choice is Tauri with a Rust backend as the security boundary and a React/TypeScript frontend as the presentation layer. This preserves strong local security while improving iteration speed, contributor accessibility, chart/table ecosystem access, and future cross-platform optionality. Tauri v2's capability model supports explicit permissions for windows and webviews, which fits a least-privilege design.[^tauri-capabilities]

2. **Keep the local encrypted vault as the non-negotiable foundation.**  
   All financial data lives in an encrypted local vault by default. The vault should unlock with a master password, optionally use Touch ID through macOS Keychain/LocalAuthentication, and never rely on a cloud login to protect local data. Apple Keychain is appropriate for small secrets such as keys and tokens, and LocalAuthentication supports biometric or passphrase-based user authentication on macOS.[^apple-keychain][^apple-localauth]

3. **Separate the desktop app from any connector relay.**  
   Automated financial connections often require API secrets, token exchange, OAuth redirect handling, and webhooks. Plaid's Link flow produces a `public_token` that is exchanged for an `access_token`, and Plaid recommends Hosted Link when official SDKs cannot be used; webview-based Link integrations are deprecated.[^plaid-link][^plaid-hosted-link][^plaid-webview] Therefore, the desktop app should not embed provider secrets. It should support manual mode first, read-only user-token connectors where possible, and optional self-hosted or managed relay modes for providers that require server-side secrets.

4. **Make Future Cash the flagship module.**  
   The MVP should prove the differentiated value: daily forward cash forecasting with deterministic events, statistical spending forecasts, credit card statement/payment modeling, income uncertainty, scenario overlays, confidence bands, and risk flags. This is the product's crown jewel.

5. **Design AI agents as scoped report generators, not chatbots.**  
   Agents should be brokered by the Rust backend, operate on explicit data views, produce schema-validated deliverables, cite local evidence, respect hard cost limits, and never gain write access to financial records.

6. **Build a thin vertical slice before building the full finance operating system.**  
   The long-term product can become a household finance OS, but the first useful release should prove one loop end-to-end: create vault, add accounts, import/enter transactions, set income/bills, reconcile balances, generate Future Cash, explain forecast changes, and backtest against actuals. Every other module must justify itself by improving this loop.

7. **Use a hybrid persistence model instead of pure event sourcing.**  
   The canonical financial truth should be normalized relational ledger tables plus immutable provenance/audit records. Keep an append-only command/operation log for idempotency, history, projection rebuilds, and future sync, but do not make a full event stream the only source of truth in the MVP. This reduces migration complexity, debugging burden, and projection-drift risk while preserving auditability.

---

## 1. Product Philosophy

### 1.1 Product Thesis

Build a **local-first personal CFO for households**.

The application should treat an individual or household like a small business: balance sheet, income statement, cash flow statement, debt schedule, investment allocation, recurring obligations, forecasted liquidity, and scenario planning.

The differentiator should not be another retrospective budgeting app. The wedge should be:

> **Forward-looking household liquidity intelligence.**

The product should help users understand what is likely to happen, not just what already happened.

### 1.2 Design Tenets, Ranked

1. **Security**  
   Encrypted local-first data, minimal cloud dependency, strong vault design, no plaintext financial secrets, no accidental telemetry, auditable open-source code, and explicit consent for anything leaving the device.

2. **Reliability**  
   Forecasts must be explainable and backtested. Bank connections must degrade gracefully. Manual entry and import must always work. The app must remain useful when a connector breaks.

3. **Usability**  
   Simple on the surface, powerful underneath. Progressive disclosure. Excellent onboarding. A daily dashboard that answers the user's next question without making them configure everything first.

4. **Flexibility**  
   Every financial story is different. Users need automated connections, manual accounts, manual corrections, file imports, document uploads, recurring overrides, custom categories, household profiles, and scenario modeling.

5. **Transparency**  
   Forecasts, categorizations, and agent reports must show their evidence and assumptions. The product should never say "AI says so" without a traceable explanation.

### 1.3 Product North Star

The north-star user experience:

> A user opens the app in the morning, unlocks their vault, and immediately understands today's cash position, upcoming bills, expected card payments, the 1-month and 3-month cash outlook, and any risks that deserve attention.

The next-level version is:

> The user also sees what changed since their last check-in, which assumptions moved the forecast, and what low-risk actions would restore their desired path.

### 1.4 Primary Personas

| Persona | Needs | Product response |
|---|---|---|
| Single salaried professional | Spending clarity, card payoff confidence, savings goals | Cash dashboard, recurring bills, card statement forecasts, savings trajectory |
| Couple/household | Shared visibility, partner accounts, fewer reauth nightmares | Household profiles, connection health, manual fallbacks, clear stale-data indicators |
| Hourly or semi-recurring earner | Paycheck uncertainty, hours planning | Expected hours entry, Bayesian paycheck updates, forecast bands |
| Contractor/freelancer | Lumpy income, quarterly tax awareness, runway | Manual expected invoices, income scenarios, cash runway, tax reserve estimates |
| Investor | Net worth and allocation clarity | Holdings, snapshots, retirement tags, alternative assets |
| Power user | Auditability, custom rules, exports, local control | Rule engine, SQLCipher vault, import/export, local model options, self-hosted connector relay |

### 1.5 Non-Negotiable Product Principles

- The app must be useful in manual-only mode.
- Financial data must be encrypted at rest before the first useful feature ships.
- A connected account is an enhancement, not a dependency.
- Forecast rows must be inspectable and editable.
- Users must be able to override categories, recurring events, assumptions, and forecast items.
- AI must not make transactions, payments, transfers, trades, or destructive edits.
- The open-source release must not happen until the security model is documented and tested.

---

## 2. Strategic Architecture

### 2.1 Recommended Architecture

Use a **local-first Tauri desktop app** built around a Rust **Finance Kernel**, an encrypted local vault, and optional connector/AI services.

The Finance Kernel is the authoritative domain layer. It owns:

- command validation
- ledger invariants
- source/provenance tracking
- import and connector commit workflows
- forecast inputs and generated outputs
- audit events
- materialized read models
- module boundaries

No frontend view, connector adapter, importer, document extractor, or AI agent may write directly to financial records. They submit typed commands or staged candidates to the kernel.

```text
macOS Desktop App, Tauri v2
  - React/TypeScript frontend in system WebView
  - Rust Finance Kernel as trusted core
  - SQLCipher encrypted local database
  - encrypted attachment store
  - command bus, domain services, append-only audit/event log
  - materialized read models for dashboard, ledger, forecast, and reports
  - local forecast/categorization/agent broker modules
  - macOS Keychain integration for small secrets
  - optional local AI runtime
  - optional connection to self-hosted connector relay

Self-Hosted Connector Relay, optional
  - Docker-deployable
  - stores provider API credentials outside the desktop app
  - handles OAuth redirects and token exchange
  - receives webhooks where supported
  - normalizes account, transaction, balance, holding, and liability data
  - syncs minimum required data back to the local app

Managed Services, optional and later
  - hosted connector relay
  - premium AI credits or hosted model access
  - license/account management if needed
  - never required for local manual use
```

### 2.2 Operating Modes

| Mode | Description | Privacy | Complexity | When it matters |
|---|---|---:|---:|---|
| Manual-only | User enters accounts, imports CSV/OFX/QFX/QIF, uploads documents | Highest | Lowest | MVP, privacy-focused users, broken connectors |
| Local app + user-token connector | SimpleFIN-style read-only token flow where user provides a token | High | Low/medium | Open-source-friendly sync without provider secrets |
| Local app + self-hosted relay | User deploys relay and brings provider credentials | High | Medium/high | Advanced users who want automation and control |
| Local app + managed relay | You operate connector infrastructure | Medium | High | Future convenience/premium offering |
| Local app + cloud AI BYOK | User brings API key for selected agent reports | Medium/high | Medium | Advanced agent reports when local model is insufficient |
| Local app + local AI | On-device categorization/reports | Highest | Medium/high | Privacy-first intelligence |

### 2.3 Trust Boundaries

The product should be designed around explicit trust boundaries.

```text
Untrusted / less trusted
  React UI, user imports, uploaded documents, transaction descriptions,
  merchant names, LLM responses, connector responses

Trusted core
  Rust command layer, vault unlock code, encryption/key management,
  database access layer, forecast engine, rule engine, agent broker

External systems
  Bank aggregators, financial institutions, AI APIs, optional relay,
  app update infrastructure
```

Rules:

- The frontend never receives raw provider credentials.
- The frontend never stores vault keys, database keys, API keys, or connector tokens.
- The frontend accesses data only through typed Tauri commands.
- Every command is permission-scoped and validated in Rust.
- Uploaded document text and transaction descriptions are treated as data, never instructions.
- LLM output is untrusted until schema-validated and safety-checked.
- WebViews are separated by trust level and capability set.
- No untrusted document, imported HTML, remote page, or raw agent output is rendered in the privileged application WebView.
- OAuth/link flows open in the system browser or a tightly scoped external-auth surface, never in the main app WebView.
- Agent reports render from typed structured blocks, not arbitrary HTML and not general-purpose Markdown. Any Markdown-like input is converted to safe report blocks by the Rust Agent Broker before rendering.

### 2.4 System Architecture Diagram

```text
+-----------------------------------------------------------------------+
|                         Tauri Desktop App                              |
|                                                                       |
|  +-----------------------------------------------------------------+  |
|  |                  React + TypeScript Frontend                    |  |
|  |                                                                 |  |
|  |  Dashboard | Future Cash | Transactions | Accounts | Planning   |  |
|  |  Documents | Agent Reports | Settings | Review Queues           |  |
|  |                                                                 |  |
|  |  No secrets. No direct DB access. No direct connector access.    |  |
|  +----------------------------+------------------------------------+  |
|                               | Tauri IPC commands                    |
|  +----------------------------v------------------------------------+  |
|  |                         Rust Trusted Core                       |  |
|  |                                                                 |  |
|  |  Auth/Vault  | Finance Kernel       | SQLCipher Data Layer        |  |
|  |  Ledger      | Import Pipeline      | Sync Engine                 |  |
|  |  Forecast    | Categorizer          | Documents                   |  |
|  |  Rules       | Risk Engine          | Agent Broker                |  |
|  |  Audit/Event Log | Read Models      | Export/Backup               |  |
|  +---------------+---------------------+---------------------------+  |
|                  |                     |                              |
|       +----------v----------+   +------v--------------------------+   |
|       | macOS Keychain      |   | Encrypted Local Vault           |   |
|       | keys/tokens/secrets |   | DB + attachments + backups      |   |
|       +---------------------+   +---------------------------------+   |
+-----------------------------------------------------------------------+
                   |                     |                 |
                   | optional             | optional         | optional
                   v                     v                 v
          Self-hosted relay        Local AI runtime       Cloud AI BYOK
          Plaid/Teller/etc.        llama.cpp/MLX/etc.    OpenAI/etc.
```

### 2.5 Why Tauri/Rust/React Beats SwiftUI for This Project

SwiftUI remains a strong macOS-native option. It has excellent Keychain, LocalAuthentication, App Sandbox, and notarization ergonomics. But for this particular project, the better practical architecture is Tauri/Rust/React, if implemented with strict security boundaries.

| Criterion | SwiftUI/native | Tauri/Rust/React | Best decision |
|---|---:|---:|---|
| macOS-native UX | Excellent | Good | SwiftUI advantage |
| Keychain/Touch ID integration | Excellent | Good via Rust/macOS bindings | Slight SwiftUI advantage |
| Open-source contributor pool | Smaller | Larger | Tauri advantage |
| Complex ledger/table UI | Moderate | Strong via AG Grid/TanStack ecosystem | Tauri advantage |
| Chart/dashboard iteration speed | Moderate | Strong | Tauri advantage |
| Future cross-platform support | Rewrite likely | Planned architecture path | Tauri advantage |
| Memory safety in trusted core | Good with Swift | Excellent with Rust | Tauri/Rust advantage |
| WebView attack surface | None | Present, but containable | SwiftUI advantage |
| Security boundary clarity | Good | Strong if Rust owns data/secrets | Tauri/Rust advantage |

Decision:

> **Use Tauri v2 + Rust core + React/TypeScript frontend as the primary architecture. Treat the WebView as presentation only and the Rust backend as the security boundary.**

Revisit native SwiftUI only if the Tauri WebView model creates unsolved accessibility, performance, App Store, sandboxing, or security obstacles.

### 2.6 Finance Kernel Boundary

The Finance Kernel is the internal API boundary for all financial state changes. It should expose typed domain commands and query views, not raw database access.

Revised persistence decision:

> Use a **hybrid ledger + operation log** model. The normalized relational ledger and domain tables are the canonical financial state. An append-only operation/audit log records every successful command, causation/correlation metadata, idempotency key, source/provenance links, and read-model projection checkpoints.

This avoids the MVP complexity of full event sourcing while preserving the benefits the plan wants: auditability, deterministic rebuilds, traceable imports, command history, and a credible path to future sync.

Kernel rules:

- Every mutation is a typed command with a stable `command_id` and idempotency key.
- Every command validates permissions, vault state, schema version, and domain invariants.
- Every successful command writes domain tables and an immutable operation/audit record in the same database transaction.
- Ledger transactions and postings are the canonical source of financial truth.
- The operation log is the canonical history of how financial truth changed.
- Read models are rebuildable from canonical tables plus operation/provenance records.
- Financial history is append-friendly; corrections use reversals, amendments, or superseding records rather than invisible mutation.
- Read-heavy screens use materialized read models projected incrementally from canonical tables.
- Forecasts and agent reports reference stable entity IDs, input snapshot IDs, and input context hashes.
- Importers and connectors produce staged candidates, not committed ledger entries.
- AI agents may create reports and proposed actions, never committed financial data.

Operation/audit log design:

- Operation records are immutable, sequentially ordered, and encrypted at rest via SQLCipher.
- Each operation has: `sequence_id`, `command_id`, `idempotency_key`, `node_id`, `hlc_timestamp`, `actor_type`, `actor_id`, `operation_type`, `affected_entities_json`, `metadata_json`, `created_at`.
- Operations carry causation IDs and correlation IDs.
- `node_id` is a stable random UUID generated once per vault instance. It identifies the device/installation that produced the operation. In single-device mode, all operations share one `node_id`. In future multi-device mode, each device has its own.
- `hlc_timestamp` is a hybrid logical clock value that combines wall-clock time with a logical counter to produce globally orderable, causally consistent timestamps without requiring clock synchronization. In single-device mode, HLC reduces to wall-clock time.
- Projection cursors record the last operation applied to each read model.
- Projection rebuild is deterministic and testable with checksums.
- Periodic table snapshots/checkpoints may be used to bound rebuild cost, but snapshots are optimization, not source of truth.
- Future multi-device sync, if ever added, should replicate operation envelopes or purpose-built sync changesets. The `node_id` and `hlc_timestamp` fields provide the minimum ordering and origin metadata required for conflict detection. Conflict resolution rules for financial records must be defined in a dedicated ADR before sync is implemented; last-write-wins is not acceptable for ledger mutations.

---

## 3. Technology Stack

### 3.1 Desktop Shell

| Layer | Recommendation | Notes |
|---|---|---|
| Desktop shell | Tauri v2 | Small binaries, Rust backend, permission/capability system, future cross-platform path |
| Notifications | Tauri notification plugin | Local-only, macOS Notification Center integration |
| Trusted backend | Rust | Owns secrets, database, forecasts, importers, sync, agents |
| Frontend | React + TypeScript strict mode | Fast iteration and broad contributor base |
| Build tooling | Vite | Fast local development |
| Routing | TanStack Router or equivalent typed router | Prefer type safety |
| Server/cache state | TanStack Query | Useful for frontend-to-Rust command result caching |
| Global UI state | Zustand or Jotai | Keep small and explicit |
| Styling | Tailwind CSS + accessible component primitives | shadcn/ui-style composition is acceptable if licenses are verified |
| Data grid | AG Grid Community or TanStack Table | Ledger performance matters; verify license and bundle size |
| Charts | Recharts, Nivo, or ECharts | Prototype several with 100k transaction scenarios |
| Forms | React Hook Form + Zod | Runtime validation and typed schemas |
| IPC schemas | serde + TypeScript type generation | Shared schema generation reduces mismatch bugs |

### 3.2 Rust Backend

| Concern | Recommendation |
|---|---|
| Async runtime | tokio where needed; keep DB operations controlled |
| SQLite access | rusqlite + SQLCipher bindings for v1; revisit sqlx only after a compatibility spike proves SQLCipher, migrations, and connection lifecycle are reliable |
| Encryption database | SQLCipher for full-database encryption |
| KDF | Argon2id for password-derived KEK; parameters stored per vault |
| Secret memory hygiene | zeroize + memsec or mlock for key material; guard pages where practical |
| Serialization | serde |
| Validation | zod on frontend, Rust validation in backend; backend is authoritative |
| HTTP | reqwest with platform TLS defaults |
| Keychain | keyring crate or direct Security framework binding |
| Logging | tracing with redaction layer and no sensitive defaults |
| Migrations | refinery, sqlx migrations, or custom signed migrations |
| Testing | cargo test, proptest, insta snapshots for deterministic outputs |
| Background jobs | Local job runner with durable encrypted job state |
| Read models | Materialized tables/views for dashboard, ledger, search, forecast, and reports |
| Observability | Local-only redacted traces, timings, counters, and failure summaries |

Database/runtime policy additions:

- Use one controlled writer path for ledger mutations; expose read-only query connections for projections and UI queries.
- Implement the database layer as a dedicated DB worker / repository boundary. Tauri commands submit Finance Kernel commands; the kernel opens explicit transactions through the DB worker. Long-running read jobs use read-only connections where safe.
- Avoid async connection-pool abstractions until SQLite/SQLCipher concurrency, WAL behavior, busy timeouts, and migration behavior are tested with fixture vaults.
- Pin SQLCipher and SQLite versions in release notes. Golden vault fixtures must include vaults created by previous app versions and previous SQLCipher versions where practical.
- Every write command must be idempotent and must commit domain-table changes, provenance links, and operation-log entries atomically.
- Use explicit transaction boundaries for imports, forecast snapshot creation, projection refresh, and backup.
- Add query budgets for user-facing screens; slow queries create local diagnostics and performance test failures.
- Prefer cursor pagination over offset pagination for large ledgers.
- Maintain migration fixtures for realistic vault sizes, not only empty-schema migrations.

SQLCipher is appropriate because it provides transparent 256-bit AES full-database encryption for SQLite.[^sqlcipher]

### 3.3 macOS Platform Integration

| Capability | Approach |
|---|---|
| Local notifications | Tauri notification plugin with explicit permission request; schedule from Rust |
| Keychain | Store only small secrets: wrapped vault keys, connector tokens, BYOK API keys if user permits |
| Touch ID | Use LocalAuthentication and Keychain access controls after initial password unlock |
| App Sandbox | Enable with minimum entitlements; restrict file/network capabilities |
| Hardened Runtime | Required for notarization; use minimum exceptions |
| Notarization | Required for trustworthy distribution outside the App Store |
| Auto-lock | App-level timer plus lock on sleep/screensaver/user switch where possible |
| Privacy mode | Optional balance hiding/blur for screenshots/screen sharing |
| File access | Security-scoped bookmarks where needed for user-selected import/backup folders |

Apple's App Sandbox limits app access to system resources and user data, hardened runtime is required for notarization uploads, and notarization increases user confidence in distributed macOS software.[^apple-app-sandbox][^apple-hardened-runtime][^apple-notarization]

### 3.3.1 Platform Abstraction

Keep macOS-specific integrations behind platform traits so the Rust Finance Kernel remains portable.

Platform services:

```rust
trait CurrencyConverter {
    fn convert(&self, amount: MoneyAmount, target_currency: CurrencyCode,
               as_of: NaiveDate) -> Result<ConvertedAmount>;
    fn aggregate(&self, amounts: &[MoneyAmount], target_currency: CurrencyCode,
                 as_of: NaiveDate) -> Result<AggregatedAmount>;
}
```

```rust
trait SecretStore {
    fn store_secret(&self, key: SecretKey, value: SecretValue, policy: AccessPolicy) -> Result<()>;
    fn load_secret(&self, key: SecretKey, prompt: AuthPrompt) -> Result<SecretValue>;
    fn delete_secret(&self, key: SecretKey) -> Result<()>;
}

trait BiometricAuth {
    fn is_available(&self) -> bool;
    fn authenticate(&self, reason: &str) -> Result<AuthResult>;
}

trait AppLifecycleLock {
    fn subscribe_lock_events(&self) -> Result<LockEventStream>;
}

trait UserFileAccess {
    fn open_user_selected_file(&self) -> Result<FileGrant>;
    fn persist_bookmark(&self, grant: FileGrant) -> Result<PersistedGrant>;
}
```

Initial implementations:

- macOS: Keychain, LocalAuthentication, security-scoped bookmarks, sleep/screensaver/user-switch lock hooks.
- Windows later: Credential Manager or DPAPI-backed equivalent.
- Linux later: Secret Service/libsecret or KWallet where available, with documented limitations.

The product should maintain a platform capability matrix so cross-platform support does not silently weaken security.

### 3.4 Forecasting and ML Stack

Start simple but structurally correct.

| Need | v1 | Later |
|---|---|---|
| Deterministic schedules | Rust date/schedule engine | Holiday calendars, payroll calendars |
| Statistical forecasts | Rust or Python prototype ported to Rust | Holt-Winters, Bayesian models, change-point detection |
| Categorization | Rules + local feature classifier | Embeddings, local model, optional LLM batch assist |
| Local AI | llama.cpp or MLX integration after core app | Apple Foundation Models where available, model abstraction |
| Experimentation | Python notebooks against synthetic data | Rust production implementations |

### 3.5 AI Provider Strategy

| Mode | Default? | Notes |
|---|---:|---|
| No AI | Yes | Core app must work without LLMs |
| Local classifier | Yes, after enough training data | Categorization and recurring detection |
| Local LLM | Optional | Privacy-first reports; hardware-dependent |
| BYOK cloud API | Optional | User supplies key; explicit consent and cost preview |
| Managed premium AI | Later | Only after strong billing/cost controls and privacy docs |

### 3.6 Headless CLI and Developer Tools

Add a small local CLI early. This is not a separate product; it is a reliability and testing tool for the same Rust crates used by the desktop app.

Initial commands:

```text
pfc vault check <vault-path>
pfc vault backup <vault-path> --out <file>
pfc import preview <vault-path> <file>
pfc forecast run <vault-path> --as-of <date> --horizon 90
pfc forecast backtest <vault-path> --from <date> --to <date>
pfc fixture generate --persona contractor --years 3
pfc projections rebuild <vault-path>
```

Rules:

- CLI commands require explicit vault unlock and never bypass the Finance Kernel.
- CLI output is redacted by default.
- CI uses the CLI against golden fixture vaults to catch regressions.
- Advanced users can later automate encrypted backups and exports through the CLI.

---

## 4. Product Scope

### 4.1 Core Modules

1. **Secure Vault**
   - Local encrypted household vault.
   - Master password unlock.
   - Optional Touch ID unlock after first unlock.
   - SQLCipher database encryption.
   - Encrypted attachment store.
   - Auto-lock and privacy mode.
   - Encrypted backup/export.
   - Read-only vault snapshot export for household partners.
   - No cloud account required.

Post-MVP Reliability Feature: **Local Notifications and Reminders**
   - Opt-in local notifications via macOS Notification Center.
   - Notification types: bill due soon, card payment approaching, balance below threshold, paycheck expected tomorrow, connector needs reauth, forecast risk flag triggered.
   - Forecast-derived notifications require Forecast Readiness above the configured threshold. If readiness is low, the app may notify about stale data or missing confirmation, but must not send precise risk claims such as "safe to spend" or "cash shortfall on date X."
   - User-configurable lead times per notification type (e.g., bills: 2 days before).
   - User-configurable quiet hours and notification categories.
   - Notification preferences stored in encrypted vault settings.
   - No cloud push infrastructure; uses Tauri notification plugin and local scheduling.
   - Clicking a notification opens the app to the relevant context (bill, account, forecast).
   - Notifications respect macOS Do Not Disturb and Focus modes.
   - Notification history stored locally for audit and replay.
   - All notification content is generated locally from forecast and risk flag data.
   - Forecast-derived notifications are disabled until Forecast Readiness, stale-data handling, and reconciliation warnings are implemented.
   - No notification content is ever sent externally.

2. **Onboarding and Setup**
   - Progressive setup based on user priorities.
   - Manual-first path.
   - Import-first path.
   - Connector path.
   - Income setup path.
   - Recurring bills setup path.
   - Dashboard preconfigured from user priorities.

3. **Accounts**
   - Cash accounts.
   - Credit cards.
   - Loans.
   - Mortgages.
   - Brokerage accounts.
   - Retirement accounts.
   - HSA/FSA.
   - Crypto wallets/exchanges.
   - Real estate.
   - Alternative/manual assets.
   - Manual liabilities.
   - Retirement/non-retirement tags.
   - Joint/individual/business/reimbursable tags.

4. **Transactions**
   - Unified ledger.
   - Splits.
   - Categories.
   - Tags.
   - Notes.
   - Attachments.
   - Merchant normalization.
   - Transfer matching.
   - Recurring detection.
   - Review queue.
   - Audit history.

5. **Income**
   - W-2 salary.
   - W-2 hourly.
   - Part-time/semi-recurring income.
   - Contractor/freelance/lumpy income.
   - Rental income.
   - Investment income.
   - Benefits/government income.
   - Pay stub upload and manual entry.
   - Expected hours input.
   - Net paycheck forecasting.
   - Confidence bands.

6. **Spending, Credit, and Debt**
   - Credit card balances.
   - Card limits and utilization.
   - Statement close dates.
   - Payment due dates.
   - Forecasted statement balances.
   - APR and interest accrual.
   - Minimum payment warnings.
   - Pay-in-full risk flags.
   - Snowball/avalanche debt planning.
   - Bill and subscription contract tracking.
   - Renewal and price-change detection.
   - Autopay source tracking.
   - Duplicate subscription review.

7. **Investments and Net Worth**
   - Holdings.
   - Allocation.
   - Historical snapshots.
   - Cost basis where available.
   - Retirement/non-retirement separation.
   - Crypto and alternative assets.
   - Real estate valuations, manual first.
   - Net worth over time.
   - Time-weighted and money-weighted return later.

8. **Future Cash**
   - Flagship view.
   - Daily forward cash ledger.
   - Left-side editable ledger.
   - Right-side trend visualization.
   - Calendar projection view.
   - 1 week, 1 month, 3 month, 6 month, 1 year horizons.
   - Known income and bills.
   - Expected variable spend.
   - Credit card statement/payment modeling.
   - One-time planned events.
   - Scenarios.
   - P10/P50/P90 forecasts.
   - Risk flags.
   - Explanation per row.

9. **Budgeting and Goals**
   - Category targets.
   - Monthly spending plans.
   - Emergency fund target.
   - Vacation fund.
   - Home down payment.
   - Baby/family planning.
   - Job transition runway.
   - Debt payoff goals.
   - Budget-to-forecast reconciliation.

10. **Documents**
    - Pay stubs.
    - Statements.
    - Bills.
    - Tax documents.
    - Receipts.
    - Insurance docs.
    - Local encrypted storage.
    - Structured extraction with review.
    - Links to transactions, accounts, income events, and agent reports.

11. **AI Agent Library**
    - Risk Agent.
    - Cash Runway Agent.
    - Planning Agent.
    - Bill/Subscription Agent.
    - Debt Agent.
    - Income Variability Agent.
    - Spending Drift Agent.
    - Vacation Affordability Agent.
    - Emergency Fund Agent.
    - Tax Reserve Estimator, later and carefully scoped.
    - Saved reports with evidence citations.

12. **Configurable Dashboard**
    - Account balances.
    - Liquid cash today.
    - Future cash mini-chart.
    - Upcoming bills.
    - Expected card payments.
    - Income forecast.
    - Spending drift.
    - Net worth.
    - Goals.
    - Risk alerts.
    - Connection health.

13. **Reconciliation Center**
    - Statement-period reconciliation.
    - Manual balance reconciliation.
    - Connector/import balance reconciliation.
    - Duplicate detection review.
    - Missing transaction search.
    - Opening/closing balance checks.
    - Month-end close checklist.
    - Reconciliation adjustments with audit trail.
    - Forecast freshness warnings based on unreconciled accounts.

14. **Money Inbox and Insight Engine**
    - One triage surface for data-quality work.
    - Review imported candidates, low-confidence categories, stale balances, connector errors, and forecast assumptions.
    - Deterministic insight cards for material forecast changes, card-payment risk, category drift, duplicate subscriptions, and reconciliation gaps.
    - Snooze, dismiss, resolve, and link-to-evidence actions.
    - LLM narration optional later; deterministic insight records are the core primitive.

15. **Financial Calendar**
    - Monthly calendar grid showing forecast events by day.
    - Color-coded event types: income (green), bills (red), card payments (orange), transfers (blue), variable spend estimates (gray).
    - Daily ending balance shown per cell, color-coded by proximity to minimum cash floor.
    - Click any day to see forecast row details and assumptions.
    - Drag-and-drop manual forecast entries to adjust timing.
    - "Danger zone" highlighting for days where P10 balance approaches minimum floor.
    - Week view for detailed daily inspection.
    - Month view for pattern recognition.
    - Reads from `cash_projection_read_model` — no new data layer required.
    - Keyboard-navigable: arrow keys move between days, Enter inspects.

---

## 5. Critical MVP Strategy

### 5.1 MVP Thesis

Do not begin with bank connections. Begin with the local encrypted vault and Future Cash engine.

Add one stricter rule: **do not begin with the full module map either.** The first product must be a thin vertical slice of the highest-value loop, not a broad but shallow finance suite.

MVP scope rule:

```text
vault -> accounts -> balances/imports -> ledger -> income/bills -> deterministic forecast
      -> forecast explanation -> reconciliation/backtest -> daily check-in
```

A feature is MVP-eligible only if it improves one of these outcomes:

- time to first useful cash forecast
- correctness of starting balances or scheduled obligations
- forecast explainability
- forecast calibration/backtesting
- safe backup/restore/dogfooding

Investments, full document extraction, cloud AI, hosted services, advanced debt tooling, and multi-account household permissions should remain visible in the plan but outside the first dogfoodable release.

#### 5.1.2 MVP Schema Profile

The data model in Section 9 is the target model, not the initial migration set. The first playable and MVP 1 must use a deliberately small schema profile.

MVP 0.5 / Week 8 schema:

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

Rule:

No target-schema table enters production migrations until the feature that owns it passes its phase gate. Target tables may exist in docs and design fixtures, but not in the live app schema.

The first MVP should prove:

> **A user can manually or semi-manually build a reliable forward cash view that is more useful than a retrospective budget dashboard.**

Automated connections improve convenience, but they are not the core product insight. They also introduce token handling, OAuth, webhook, pricing, rate limit, institution reliability, and support burdens.

### 5.1.1 Week 8 Kill Switch: First Playable

The plan's highest-likelihood risk is solo developer motivation decay. The most effective countermeasure is an aggressive first-playable deadline.

**By Week 8, the developer must be able to use the app for personal financial planning.**

Week 8 first-playable scope (non-negotiable minimum):

- Tauri app opens and creates an encrypted vault.
- Password unlock works.
- Manual accounts with balances.
- Manual transaction entry (no import).
- Manual recurring income schedule (one type: salary).
- Manual recurring bills (flat list, no contracts).
- Deterministic Future Cash ledger: starting cash + income - bills = daily balances.
- Ugly but functional dashboard showing: liquid cash, upcoming bills, upcoming income, 30-day cash forecast line.
- Encrypted backup to file.

Real-data safety gate:

The developer must not use a real personal-finance vault until all of the following pass on synthetic fixture vaults:

- encrypted backup export
- restore into a fresh app instance
- wrong-password failure test
- log redaction test
- no plaintext database open with normal SQLite tools
- no plaintext attachment preview artifacts outside the vault
- basic migration rollback/repair test
- manual recovery instructions written and tested

What is explicitly NOT in Week 8:

- CSV/OFX/QFX/QIF import (MVP 1, Week 10)
- Touch ID (MVP 1, Week 10)
- Category taxonomy beyond a flat hardcoded list (MVP 1, Week 10)
- User rules (MVP 1.5)
- Reconciliation (MVP 1.5)
- Attachments (MVP 1.5)
- Split transactions (MVP 1.5)
- Merchant normalization (MVP 2)
- Confidence bands (MVP 2)
- Risk flags (MVP 2)

If Week 8 is not met, cut scope further — do not extend the timeline. The app must be personally useful before the developer's available focus time contracts significantly in August.

Decision rule: if the deterministic forecast proves useful in personal dogfooding by Week 8, continue to MVP 1. If it doesn't, reconsider whether the product thesis is correct before investing further.

### 5.2 MVP 1: Local-First Manual Prototype

MVP 1 (Weeks 9-12, building on Week 8 first-playable) should add:

- CSV import with column mapping.
- Basic OFX/QFX/QIF import if feasible.
- Optional Touch ID unlock after password unlock.
- Full transactions ledger with search/filter.
- Category taxonomy.
- Basic credit card due-date modeling.
- Encrypted backup/export with restore verification.
- Quick-entry templates from recurring events.
- No external connectors.
- No cloud AI.

Exit criteria:

- A user can create a vault, enter accounts, import transactions, set income and bills, and see a daily cash forecast for up to 1 year.
- The app remains fully functional offline.
- No sensitive data appears in plaintext logs, crash files, or unencrypted storage.
- Forecast rows can be explained and edited.

### 5.3 MVP 2: Learning and Forecasting Depth

MVP 2 should include:

- Recurring transaction detection.
- Rule-based categorization.
- Merchant normalization.
- User override learning.
- Statistical variable spending estimates.
- Basic confidence bands.
- Risk flags.
- Credit card statement forecast.
- Forecast backtesting against historical periods.

Exit criteria:

- The app can generate useful forecasts from historical transactions.
- Risk flags are evidence-backed.
- Forecast error can be measured and improved.

### 5.4 MVP 3: Connector Adapters

Only after the manual app is useful:

- Add SimpleFIN adapter or equivalent read-only user-token connector.
- Add Teller adapter if practical for independent/open-source usage.
- Add Plaid BYOK/self-hosted relay adapter.
- Add connector health dashboard.
- Add stale-data and reauth workflows.
- Add dedupe between imported and synced transactions.

Exit criteria:

- Connected data is convenient, but not required.
- A stale or broken connection does not break forecasting.
- The app never embeds provider secrets in public source code.

---

## 6. Local Security and Privacy Architecture

### 6.1 Security Goal

The security goal is not merely to hide the UI behind a login screen. The goal is:

> **Without the vault password or authorized biometric/keychain unlock, local financial data should remain cryptographically inaccessible.**

### 6.2 Vault Design

Recommended flow:

1. User creates a vault password.
2. App generates a random Data Encryption Key, or DEK.
3. App derives a Key Encryption Key, or KEK, from the password using Argon2id.
4. App stores the KDF parameters with the vault metadata.
5. App encrypts/wraps the DEK with the KEK.
6. SQLCipher uses a database key derived from the DEK.
7. Attachments use per-file encryption keys.
8. Per-file keys are wrapped by the vault key.
9. Optional Touch ID stores a wrapped vault unlock secret in Keychain with biometric access control.
10. The password is never stored.
11. Unlock succeeds by decrypting and authenticating a known vault metadata blob.

Additional requirements:

12. The vault uses a versioned encryption envelope so future KDF, key-wrap, and attachment encryption changes can be migrated safely.
13. The app explicitly configures and tests SQLite/SQLCipher journal, WAL, SHM, temp-store, and backup behavior.
14. No decrypted attachment, OCR text, statement preview, or export artifact may be written outside the encrypted vault unless the user explicitly exports it.
15. The app maintains a vault manifest containing encrypted file inventory, content hashes, schema version, app version, and backup compatibility metadata.
16. Optional recovery key support should be designed before public release and externally reviewed before being enabled by default.
17. Key material directly controlled by the app (DEK, KEK, per-file keys, unwrapped secrets) should use guarded allocations, zeroization, and memory-locking where supported. Document OS and library limitations.
18. On vault lock or app exit, app-controlled key buffers must be zeroized. The implementation must minimize copies across FFI, IPC, logging, panic, and serialization boundaries.
18a. The security docs must explicitly state that local malware running as the user can potentially read process memory after unlock. The app's main cryptographic promise is protection at rest when the vault is locked or the device storage is offline.
19. Tauri IPC responses must never include raw key material, unwrapped secrets, or intermediate cryptographic state. The IPC serialization boundary is a trust transition from protected Rust memory to unprotected WebView JavaScript heap.
20. macOS crash reports should be tested to verify they do not contain key material. The hardened runtime's `com.apple.security.cs.disable-library-validation` exception should not be used unless absolutely necessary.

### 6.2.1 Vault State Machine and Startup Self-Test

Treat vault access as an explicit state machine, not a collection of unlock helpers.

```text
NoVault -> CreatingVault -> Locked -> Unlocking -> Unlocked
        -> Locking -> Locked
        -> Rekeying -> Unlocked
        -> Migrating -> Unlocked
        -> RestoringBackup -> Locked/Unlocked
        -> CorruptNeedsRecovery
```

Startup self-test should verify:

- vault manifest is present and decryptable after unlock
- schema version and migration history are coherent
- SQLCipher keying succeeds before any query path is exposed
- attachment directory manifest matches database metadata
- WAL/SHM/temp configuration matches the security policy
- last shutdown did not leave an incomplete migration, backup, or key rotation
- read-model checksums are current or safely rebuildable

The UI should expose vault-health failures as recovery flows, not generic startup errors.

### 6.3 KDF Guidance

Use Argon2id with per-vault random salt and versioned parameters. The app should support calibrated profiles rather than one hard-coded setting.

```text
algorithm: Argon2id
profiles:
  interactive_default:
    target_latency: 500ms-1000ms on supported hardware
    minimum_memory: 64 MiB
    preferred_memory: highest calibrated value within latency and memory budget
  high_security:
    user-selectable, slower unlock, higher memory budget
  legacy_compatibility:
    only for old vaults until rekey
parallelism: calibrated per device
salt: random per vault
params stored: yes, in vault metadata
```

Rules:

- KDF parameters must be versioned.
- Existing vaults should support rekey/parameter upgrade.
- Wrong-password handling must not leak timing-sensitive metadata beyond unavoidable unlock failure.
- Failed unlock attempts should be locally throttled.

### 6.4 SQLCipher and Attachments

Use SQLCipher for the primary database. Store large files outside the database in an encrypted attachment directory.

```text
vault/
  vault.db                SQLCipher encrypted database
  vault.db-wal            if WAL is enabled, covered by SQLCipher configuration and tests
  vault.db-shm            if WAL is enabled, treated as sensitive operational state
  attachments/
    ab/cd/<id>.blob       encrypted attachment blob
  manifests/
    vault-manifest.json.enc
  exports/
    optional encrypted export packages
```

Attachment metadata lives in the database. Attachment contents are encrypted separately to avoid database bloat and make file streaming practical.

Attachment rules:

- Use per-attachment random content encryption keys.
- Store attachment blobs by opaque ID, not original filename.
- Keep original filenames only as encrypted metadata.
- Encrypt OCR text and extraction outputs as sensitive data.
- Never use the OS temporary directory for plaintext document processing.
- Test that common preview, OCR, and crash flows do not leave plaintext artifacts.

### 6.5 Keychain and Touch ID

Keychain use:

- Wrapped vault unlock secret for biometric unlock.
- Connector tokens where local storage is allowed.
- BYOK AI API keys if the user explicitly chooses to save them.
- Relay credentials.
- Optional recovery-key verifier metadata, never the recovery key itself.

Do not store:

- Raw master password.
- Unwrapped long-lived database key without access controls.
- Plaintext provider secrets in config files.

Recovery rules:

- The master password remains the primary unlock path.
- Optional recovery key is generated with high entropy and shown once.
- Recovery key wraps the vault DEK independently from the password-derived KEK.
- Recovery-key unlock should require explicit user action and should rotate/re-wrap keys afterward.
- Losing both password and recovery key means unrecoverable data.

Touch ID rules:

- First unlock after vault creation or password change requires password.
- Biometric unlock is optional.
- User can revoke biometric unlock.
- Password remains the ultimate recovery path.
- If password is lost and no valid encrypted recovery/export exists, the data is unrecoverable.

### 6.5.1 Emergency Access Design (Reserve Key Slot Only; Do Not Implement Early)

Emergency access is important for household reality, but it is also one of the easiest ways to weaken a local-first vault. The early plan should reserve space in the key hierarchy without implementing the user-facing feature.

Revised design constraints:

- The vault key hierarchy reserves an optional emergency-access key-wrap slot for future use.
- No emergency contact can be configured in MVP or private dogfood builds.
- A waiting-period design requires a reliable notification and denial channel. In a purely local app, a 72-hour waiting period is mostly UX theater unless there is a trusted communication path.
- Any implementation must define revocation semantics, notification reliability, abuse cases, social-engineering risks, and recovery procedures before code ships.
- External cryptographic/security review is required before this feature is enabled.

For the first serious dogfood release, prefer: strong password education, encrypted backups, optional high-entropy recovery key, and clear household estate-planning guidance.

Add an intermediate feature: Recovery Packet.

Recovery Packet properties:

- Generated only after explicit user action.
- Contains no plaintext financial data.
- Includes vault identifier, backup instructions, restore instructions, app/version compatibility notes, and user-written household notes.
- May include an encrypted recovery-key envelope if recovery keys are enabled.
- Can be printed or exported as an encrypted PDF/JSON bundle.
- Does not contact any server.
- Does not notify emergency contacts.
- Does not bypass the vault password/recovery-key model.
- Regeneration invalidates prior packet metadata if keys are rotated.

### 6.6 Logging Policy

Default logging must be safe for sharing in bug reports.

Never log:

- Account numbers.
- Full transaction descriptions.
- Balances.
- Provider tokens.
- API keys.
- Vault keys.
- Passwords.
- Raw document text.
- Agent prompts containing financial data.
- LLM responses containing sensitive data, unless stored intentionally as encrypted agent reports.

Allowed logs:

- Event types.
- Error codes.
- Module names.
- Timings.
- Counts.
- Hashes or redacted IDs where necessary.

### 6.6.1 Local Observability Policy

The app should collect local-only operational telemetry for reliability, with export disabled by default.

Allowed local metrics:

- job duration
- import row counts
- dedupe candidate counts
- forecast generation duration
- query duration buckets
- UI render timing buckets
- connector status codes, redacted
- retry counts
- schema migration duration
- backup/restore success or failure

Rules:

- Telemetry is stored inside the encrypted vault or ephemeral memory.
- No telemetry leaves the device unless the user previews and explicitly exports it.
- Diagnostic export must have redacted and full-local modes.
- Full-local mode remains encrypted and user-controlled.
- Redaction tests are part of CI.

### 6.7 Privacy Modes

Features:

- Hide balances mode.
- Blur dashboard mode.
- Copy-safe values, opt-in.
- Redacted export for support.
- No telemetry by default.
- Optional diagnostics must be local-previewed before sending.

### 6.8 Network Policy

Default:

- No network access required for manual mode.
- No external requests until user explicitly configures connector, update check, or AI provider.
- The app should make network destinations inspectable in settings.
- Consider a network activity log with redacted metadata.

Certificate pinning:

- Do not blindly pin third-party aggregator certificates by default; it can create fragile outages when providers rotate infrastructure.
- Use platform TLS validation and provider-recommended SDK/API practices.
- Pin only first-party relay endpoints if operationally supportable.

### 6.9 App Hardening

- Tauri capabilities deny by default.
- No arbitrary shell execution.
- No remote frontend assets.
- Strict Content Security Policy.
- No inline scripts unless unavoidable and justified.
- Disable devtools in production builds unless explicitly enabled in debug.
- Validate every IPC command in Rust.
- Use App Sandbox with minimum entitlements.
- Use hardened runtime and notarized distribution.
- Sign updates.
- Verify update signatures before applying.
- Define one Tauri capability file per window/webview role.
- Add a release-build capability audit that fails CI if:
  - any production capability uses wildcard windows
  - any production capability grants broad filesystem or shell access
  - any window/webview belongs to multiple capability files without an explicit security exception
  - `withGlobalTauri` is enabled in production
  - devtools are enabled in production
  - remote frontend assets are allowed in production
  - the main window can navigate to arbitrary remote URLs
  - document preview or agent report windows have write-capable IPC
  - CSP contains unsafe-inline or unsafe-eval without a documented exception
- Keep the main app window local-only and privileged only for the minimum required commands.
- Disable `withGlobalTauri` in production.
- Forbid arbitrary navigation from the main app WebView.
- Block remote frontend assets in production.
- Render untrusted document previews in a separate no-IPC preview surface or via OS-level preview where practical.
- Sanitize SVG, markdown, HTML-like document text, and LLM-generated markdown before rendering.
- Treat merchant names, transaction descriptions, PDF text, and imported filenames as potentially hostile strings.

Release/update key management additions:

- Store update-signing private keys outside the developer workstation when practical.
- Document key rotation and loss procedures before enabling auto-update.
- Keep notarization credentials, update signing keys, and app signing identities separate.
- CI should verify release artifacts, signatures, checksums, and SBOM generation in one reproducible release workflow.

Recommended window/capability model:

| Surface | Content | Tauri IPC | Network | Notes |
|---|---|---:|---:|---|
| `main` | app UI | limited typed commands | off by default | no remote navigation |
| `document_preview` | PDF/image/text preview | none or read-only narrow IPC | off | untrusted content |
| `agent_report` | sanitized report blocks | read-only report fetch | off | no raw HTML |
| `external_auth` | connector auth return handling | narrow callback only | provider allowlist | prefer system browser |
| `debug` | development tooling | debug only | debug only | never in release |

---

## 7. Threat Model

Maintain `docs/security/threat-model.md` from day one.

### 7.1 Primary Threats

| Threat | Example | Mitigation |
|---|---|---|
| Lost or stolen Mac | Attacker has filesystem access | SQLCipher vault, strong KDF, no plaintext cache |
| Malware on device | Malware reads app files or memory | Encryption at rest, Keychain controls, mlock/guard pages for key material, minimal secrets in memory window, OS updates |
| Swap-file exposure | Key material paged to unencrypted swap | mlock on key buffers, madvise(MADV_DONTDUMP), FileVault recommendation in docs |
| Core dump exposure | Crash dump contains key material | Hardened runtime, MADV_DONTDUMP, crash dump testing in CI |
| Compromised frontend | XSS or dependency bug in WebView | Rust backend trust boundary, Tauri capabilities, CSP, no raw secrets in UI |
| Malicious import | Crafted CSV/OFX/PDF exploits parser | Safe parsers, sandboxed parsing where possible, fuzzing, size limits |
| Prompt injection | Statement says "ignore instructions" | Treat imported text as data only; structured agent inputs; no tool autonomy |
| Connector token leak | Provider token exposed | Keychain, relay token encryption, no logs, revocation flow |
| Relay compromise | Self-hosted relay leaks credentials | Minimize stored data, encrypt tokens, document hardening, support local/manual mode |
| Supply-chain compromise | Malicious dependency update | Lockfiles, SCA, secret scanning, SBOM, code review |
| Update compromise | Malicious app update | Signed releases, checksums, notarization, update signature verification |
| Forecast error | User relies on bad projection | Confidence intervals, assumptions, explanations, backtesting, disclaimers |
| Household privacy conflict | Partner sees hidden account | Household permissions later; local profiles; clear data ownership model |
| Data loss | User loses password or disk | Encrypted backups, recovery education, export reminders |

### 7.2 Security Standards and Control Frameworks

There is no single universal security rating for a local-first personal finance desktop application. Use a control-mapping strategy:

| Framework | Use |
|---|---|
| NIST SSDF SP 800-218 | Secure development lifecycle baseline |
| NIST CSF 2.0 | Risk-management vocabulary and governance |
| OWASP ASVS | Web/API-style controls relevant to Tauri frontend and relay |
| OWASP MASVS | Client/app security reference, especially data storage and privacy concepts |
| OWASP SCVS | Supply-chain verification and dependency controls |
| CIS Controls | Operational hardening reference |
| SOC 2 principles | Useful later for managed services, not required for local-only app |

NIST SSDF defines secure software development practices, OWASP ASVS defines application security requirements, OWASP MASVS is a mobile app security standard, and OWASP SCVS focuses on software supply-chain risk controls.[^nist-ssdf][^owasp-asvs][^owasp-masvs][^owasp-scvs]

---

## 8. Data Integration Strategy

### 8.1 Ingestion Paths

Support these paths in priority order:

| Priority | Path | Purpose |
|---:|---|---|
| 1 | Manual entry | Full offline functionality and corrections |
| 2 | File import | CSV, OFX, QFX, QIF, brokerage CSV, crypto CSV |
| 3 | Document upload | Pay stubs, bills, statements, receipts, tax docs |
| 4 | User-token connector | SimpleFIN-style read-only sync where practical |
| 5 | Direct provider adapter | Teller or similar where terms and reliability work |
| 6 | Self-hosted relay | Plaid/MX/Mastercard-style integrations needing server-side secrets |
| 7 | Managed relay | Future convenience/premium option |

### 8.1.1 Ingestion Pipeline

All ingestion paths should use the same commit pipeline:

```text
raw source input
  -> encrypted source record
  -> parser/extractor run
  -> normalized staged candidates
  -> dedupe/conflict detection
  -> user/rule review
  -> kernel commit plan
  -> ledger postings + provenance links
  -> materialized read model refresh
```

Rules:

- Importers, document extractors, and connectors do not write committed ledger records directly.
- Every import/sync has an import batch ID.
- Every parsed row or provider object has a source record ID and source hash.
- Every committed transaction can be traced back to source records, user edits, rules, or manual entry.
- Import batches can be previewed, partially committed, rolled back before commit, or superseded after commit.
- Dedupe decisions are stored and reviewable.

This pipeline is required before serious connector work.

### 8.1.2 Parser and Document Isolation

Treat import and document parsing as a hostile-input boundary. CSV, OFX/QFX/QIF, PDFs, images, and zipped exports can contain malformed, oversized, or adversarial payloads.

Recommended design:

- Run high-risk parsers in a separate local worker process or narrowly scoped Rust task with explicit memory, file-size, page-count, and time limits.
- Parser workers receive bytes and return staged candidates; they do not receive vault keys, database handles, network access, or frontend IPC.
- Store raw source bytes encrypted before parsing.
- Detect and reject zip bombs, huge PDFs, excessive image dimensions, deeply nested archives, malformed encodings, and suspicious embedded active content.
- Keep parser crash reports redacted and local.
- Add fuzz tests and adversarial fixture files for every parser.

### 8.1.3 Importer Plugin Architecture

File importers should implement a standard trait so new institution formats can be added
as separate crates without modifying the core ingestion pipeline.

```rust
trait ImporterPlugin {
    fn plugin_id(&self) -> ImporterPluginId;
    fn display_name(&self) -> &str;
    fn supported_extensions(&self) -> &[&str];
    fn detect_confidence(&self, sample: &[u8], filename: &str) -> DetectionConfidence;
    fn parse(&self, input: ImportInput) -> Result<Vec<StagedCandidate>>;
    fn column_mapping_hints(&self) -> Option<ColumnMappingHints>;
    fn institution_hint(&self) -> Option<&str>;
}
```

Design rules:

- Importers are compiled into the binary; no runtime dynamic loading.
- Importers receive read-only byte slices; they have no access to the vault, database, or network.
- Importers produce `StagedCandidate` records that flow through the standard ingestion pipeline.
- Auto-detection runs all registered importers' `detect_confidence` and offers the best match.
- Community-contributed importers live in `crates/importers/` as separate sub-crates.

Importer requirements:

- Amount parsing must be locale-aware and previewable.
- Date parsing must preserve raw value, parsed date, and parser confidence.
- Importers must support separate debit/credit columns, signed amount columns, pending/posted status columns, and institution-specific reversal conventions.
- Every parsed amount/date/description field retains raw source text for review.
- Ambiguous amount or date parsing produces a staged warning, not a silent commit.
- Import fixtures must include negative amounts, parentheses, thousands separators, non-USD currencies, duplicate rows, pending-to-posted transitions, and malformed rows.
- Each importer crate includes test fixtures from anonymized/synthetic data for the target format.
- The importer registry is a static array built at compile time.

Initial importers to implement:

- Generic CSV with column mapping (built-in)
- OFX/QFX (built-in)
- QIF (built-in)
- Chase CSV
- Schwab brokerage CSV
- Fidelity CSV
- Coinbase CSV
- Mint export CSV
- YNAB export CSV

### 8.2 Connector Provider Strategy

| Provider/path | Role | Notes |
|---|---|---|
| Manual + files | Required | Always available fallback |
| OFX/QFX/QIF | Required early | Many institutions still provide exports; parser quality matters |
| SimpleFIN | Early candidate | Read-only protocol; can use SimpleFIN Bridge when an institution lacks a SimpleFIN Server.[^simplefin] |
| Teller | Candidate adapter | Exposes accounts, balances, transactions, and bank account connection flows; evaluate pricing, institution coverage, and terms.[^teller] |
| Plaid | Later adapter | Strong coverage, but requires careful token/server handling and Hosted Link/external flow design |
| MX | Later/enterprise | Useful but likely less practical for early OSS |
| Mastercard Open Banking | Later | Enterprise/commercial path |
| FDX-aligned APIs | Future | Watch U.S. open banking evolution; FDX recognized by CFPB as a standard-setting body.[^fdx-cfpb] |

The current U.S. open banking environment remains legally and operationally unsettled. 12 CFR Part 1033 remains the published regulatory text, but CFPB compliance dates have been stayed and the Bureau has opened reconsideration of possible amendments. Treat open-banking timelines, data coverage, and third-party obligations as volatile planning inputs, not MVP dependencies.[^ecfr-1033][^cfpb-reconsideration]

### 8.2.1 Connector Capability, Cost, and Terms Registry

Before implementing provider adapters, maintain a provider registry that captures operational and legal constraints. This prevents provider-specific assumptions from leaking into the product and avoids surprise support burdens.

```text
connector_provider_registry
  provider_id
  auth_modes_supported
  requires_server_secret bool
  supports_transactions bool
  supports_balances bool
  supports_holdings bool
  supports_liabilities bool
  supports_webhooks bool
  supports_item_revocation bool
  known_reauth_behavior_json
  pricing_model_summary
  data_retention_constraints_json
  terms_reviewed_at
  regulatory_status enum: stable, changing, stayed, litigation, unknown
  ui_auth_constraints_json
  desktop_supported_auth_surface enum: system_browser, hosted_link,
                                       native_sdk_unavailable, blocked
  adapter_enabled_by_default bool
  risk_level enum: low, medium, high, blocked
  notes
```

Rules:

- No adapter is enabled by default until its terms, auth model, data retention, and revocation path are documented.
- Provider capability differences are surfaced in Settings and connection-health UI.
- If a provider changes pricing, terms, or authentication flow, the adapter can be disabled without breaking manual/import mode.
- Connector adapters are feature-flagged and disabled by default until terms, auth surface, revocation, data retention, pricing, and desktop UX constraints are reviewed.
- Plaid-style flows must use Hosted Link/system-browser style integration where provider guidance discourages embedded WebViews.

### 8.3 Provider-Neutral Connector Interface

Do not let Plaid, Teller, or SimpleFIN concepts leak into the core schema.

```rust
trait ConnectorProvider {
    fn provider_id(&self) -> ProviderId;
    async fn create_link_session(&self, request: LinkSessionRequest) -> Result<LinkSession>;
    async fn exchange_or_register_token(&self, request: TokenExchangeRequest) -> Result<ConnectorItem>;
    async fn sync_accounts(&self, item: ConnectorItemRef) -> Result<Vec<NormalizedAccount>>;
    async fn sync_balances(&self, item: ConnectorItemRef) -> Result<Vec<NormalizedBalance>>;
    async fn sync_transactions(&self, item: ConnectorItemRef, cursor: SyncCursor) -> Result<TransactionSyncPage>;
    async fn sync_holdings(&self, item: ConnectorItemRef) -> Result<Vec<NormalizedHolding>>;
    async fn sync_liabilities(&self, item: ConnectorItemRef) -> Result<Vec<NormalizedLiability>>;
    async fn connection_health(&self, item: ConnectorItemRef) -> Result<ConnectionHealth>;
    async fn revoke(&self, item: ConnectorItemRef) -> Result<()>;
}
```

### 8.4 Self-Hosted Connector Relay

Required for providers that cannot be safely used from the desktop client.

Relay responsibilities:

- Store provider API credentials.
- Handle OAuth redirect URIs.
- Exchange public tokens for access tokens.
- Receive webhooks.
- Normalize provider-specific payloads.
- Encrypt connector tokens at rest.
- Return only required normalized data to desktop.
- Return signed sync manifests with relay instance ID, desktop pairing ID, monotonic batch sequence, nonce, expiration timestamp, provider cursor, item health, normalized record hashes, and schema version.
- Support a minimal-stateless mode where feasible.
- Avoid storing full transaction history unless required for provider cursoring or explicitly enabled.
- Pair with the desktop app using a one-time pairing code that establishes a device-specific relay public key and desktop client public key.
- Store the trusted relay public key in the encrypted vault and/or platform secret store according to the local security policy.
- Reject unsigned, expired, replayed, out-of-order, wrong-relay, or wrong-item batches.
- Support relay key rotation with an explicit user-visible trust prompt.
- Support per-connector-item revocation and relay unpairing.
- Provide a local/private deployment option.
- Expose health endpoints.
- Support token revocation.

Relay non-goals:

- It should not become the user's primary financial database.
- It should not store full historical transaction data unless explicitly necessary.
- It should not require access to the local vault key.
- It should not be required for manual mode.
- It should not become the user's audit log.
- It should not be the durable source of household financial history.
- It should not render provider auth inside the main app WebView.

Relay sync contract:

```text
desktop requests sync
  -> relay performs provider interaction
  -> relay returns signed normalized batch manifest
  -> desktop stores encrypted source batch
  -> desktop stages candidates
  -> desktop dedupes/reconciles
  -> user/rules commit to ledger
```

Connector batches must flow through the same ingestion pipeline as file imports.

Replay-protection rules:

- Each relay batch has `relay_instance_id`, `desktop_pairing_id`, `connector_item_id`, `batch_sequence`, `nonce`, `issued_at`, and `expires_at`.
- The desktop stores the last accepted sequence per connector item.
- Replayed or older sequences are rejected and create a connector security event.

### 8.5 Reauthentication and Reliability Strategy

Acknowledge reality: some reauthentication pain is caused by banks, MFA, aggregator policy, risk engines, and OAuth expiration. The product cannot eliminate all of it.

The product promise should be:

> **We make connection health visible, recovery easy, batchable where possible, and forecasts resilient when connections are stale.**

Features:

- Connection health dashboard.
- Last successful sync timestamp.
- Institution-level status.
- Data freshness badges.
- Stale data warning in forecasts.
- Batch reauth for same institution where provider supports it.
- Clear user-facing error messages.
- Manual balance override when connection is stale.
- Import fallback.
- Forecast continues using last known data plus assumptions.
- Partner-friendly reauth workflow notes.
- Calendar reminders for known reauth cycles.
- Sync dedupe and reconciliation.
- External-browser or hosted-link auth where provider guidance requires it.
- Legal/regulatory watchlist for provider terms, open banking rules, screen-scraping restrictions, and data retention requirements.

Bad message:

```text
Connection error. Try again.
```

Good message:

```text
Chase requires reauthorization for this connection. The last successful sync was Apr 28, 2026. Your forecast is currently using the last known balance plus manually entered events. Reconnect, import a file, or enter a manual balance.
```

---

## 9. Data Model

Use an append-friendly, ledger-native model with audit trails for imported/edited financial data. Do not mutate financial history invisibly.

### 9.1 Core Principles

- Use integer minor units for money plus currency code and currency exponent. Avoid `amount_cents` naming in canonical schemas because not all currencies or assets are cent-based.
- All monetary aggregation across accounts must pass through a currency conversion layer.
- The household has a `reporting_currency` (default USD) used for dashboard totals, net worth, and forecast summaries.
- Exchange rates are stored locally and updated manually or via optional rate feed.
- Aggregation queries never silently sum amounts in different currencies without conversion.
- Represent canonical financial movement as ledger transactions plus postings against an explicit ledger-account registry. User-visible accounts are not the only posting targets; system virtual accounts are required for income, expenses, equity/opening balances, adjustments, and clearing flows.
- User-facing transactions are projections over ledger records, not the only source of truth.
- Add a ledger-account substrate:
  - `accounts` are user-visible financial containers: checking account, credit card, mortgage, brokerage account, manual cash wallet, etc.
  - `ledger_accounts` are posting targets. Some map one-to-one to user-visible accounts. Others are system virtual accounts.
  - Every committed posting points to a `ledger_account_id`.
  - User categories may map to expense/income ledger accounts, but category labels are not a substitute for balanced postings.
  - Opening balances are equity postings, not mutable account fields.
  - Balance observations from imports/connectors/statements are evidence used for reconciliation, not canonical account truth by themselves.
- Transfers, credit card payments, loan payments, reimbursements, and split purchases should be represented as multiple postings rather than special-case flags.
- Use reversals/amendments for corrections instead of destructive edits where financial meaning matters.
- Store original provider/import payload hashes for dedupe.
- Separate current balance from balance snapshots.
- Preserve user overrides.
- Track source of truth for every record.
- Prefer soft deletion for non-canonical records; for canonical financial movements, use reversals/amendments.
- Version forecast assumptions and agent inputs.

### 9.1.1 Date and Timezone Policy

Financial dates should follow these rules:

- All dates stored in the database are calendar dates (DATE type, no time component)
  representing the date the user considers the transaction to belong to.
- The vault stores a `household_timezone` setting (IANA timezone, e.g., `America/Los_Angeles`).
- Timestamps (created_at, updated_at, sync times) use UTC.
- Provider/connector dates are normalized to the household timezone during ingestion.
- The forecast engine operates on calendar dates in the household timezone.
- Recurring event schedules resolve to calendar dates in the household timezone.
- When a provider returns a UTC timestamp for a transaction, the ingestion pipeline
  converts it to a calendar date in the household timezone.
- Users may override any transaction date.
- The system does not attempt to infer merchant timezones.
- Day boundaries for "today," "this month," and forecast horizons use the household timezone.

### 9.1.2 Identity, Idempotency, and Command Safety

Use stable opaque IDs and idempotency everywhere financial records can be created.

Rules:

- Prefer UUIDv7/ULID-style sortable IDs for local entities and operation records.
- Every mutating command has `command_id`, `idempotency_key`, `actor_id`, `vault_schema_version`, `node_id`, `hlc_timestamp`, `causation_id`, and `correlation_id`.
- Import commits derive deterministic idempotency keys from source batch ID, source record IDs, normalized payload hashes, and commit-plan version.
- Connector sync commits derive idempotency keys from provider item, cursor/window, normalized record hashes, and commit-plan version.
- Retrying a command must return the original result or a safe no-op, never duplicate ledger records.
- IDs shown to users should be short display IDs, not database primary keys.

Add tables:

```text
command_idempotency_keys
  idempotency_key
  command_id
  command_type
  result_ref_json
  created_at
  expires_at nullable

operation_log
  sequence_id
  command_id
  idempotency_key
  node_id
  hlc_timestamp
  operation_type
  affected_entities_json
  metadata_json
  created_at

projection_cursors
  projection_name
  last_sequence_id
  checksum
  rebuilt_at nullable
  status enum: current, stale, rebuilding, failed
```

### 9.2 Entity Overview

```text
vault_metadata
operation_log
command_idempotency_keys
projection_cursors
read_model_checksums
households
household_members
profiles
accounts
ledger_accounts
system_ledger_accounts
balance_observations
institutions
exchange_rates
exchange_rate_sources
connector_items
connector_events
connector_provider_registry
balances
balance_snapshots
transaction_display_rows_read_model
split_groups
split_lines
categories
category_aliases
tags
transaction_tags
merchant_aliases
merchant_identities
recurring_events
recurring_event_instances
income_sources
income_events
paystub_documents
bills
credit_card_cycles
credit_card_statements
loans
loan_payment_schedules
investment_holdings
investment_transactions
investment_snapshots
asset_prices
manual_assets
manual_liabilities
documents
document_extractions
forecast_runs
forecast_rows
forecast_assumptions
risk_flags
scenarios
scenario_events
agent_definitions
agent_runs
agent_evidence
user_rules
categorization_examples
review_queue_items
change_journal_entries
audit_events
settings
source_records
source_batches
parser_runs
staged_transactions
staged_accounts
staged_balances
dedupe_decisions
provenance_links
reconciliation_sessions
bill_contracts
forecast_input_snapshots
forecast_model_registry
forecast_backtest_results
forecast_actuals
forecast_quality_scores
data_quality_scores
```

### 9.2.1 Exchange Rates

```text
exchange_rates
  id
  base_currency
  quote_currency
  rate_decimal text       -- stored as decimal string to avoid floating point
  rate_minor_multiplier   -- integer multiplier for minor-unit conversion
  effective_date
  source enum: manual, api_feed, connector, system_default
  source_id nullable
  created_at

exchange_rate_sources
  id
  name
  type enum: manual, ecb, openexchangerates, coinapi, other
  api_key_ref nullable    -- references Keychain if stored
  last_fetched_at nullable
  fetch_frequency enum: manual, daily, weekly
  is_active bool
  created_at
  updated_at
```

Rules:

- The household's `reporting_currency` determines the denomination for all aggregated views (dashboard totals, net worth, forecast summaries, goal progress).
- Exchange rates are immutable records keyed by date. Rate updates create new records.
- When no rate exists for a specific date, the system uses the most recent prior rate.
- The system ships with a `system_default` USD/USD = 1.0 rate.
- For MVP, exchange rates are manual-entry only. Optional API feed support is post-MVP.
- Forecast rows in non-reporting currencies are converted at the latest available rate with an explicit "exchange rate assumed" note in the explanation.
- Net worth snapshots record the exchange rates used at snapshot time for auditability.
- Currency conversion is never silent: any aggregation across currencies must be traceable to a specific rate record.

### 9.3 Accounts

```text
accounts
  id
  household_id
  owner_profile_id nullable
  name
  institution_id nullable
  institution_name
  type enum: checking, savings, cash, credit_card, brokerage,
             retirement_401k, retirement_403b, retirement_ira_traditional,
             retirement_ira_roth, hsa, crypto_wallet, real_estate,
             other_asset, loan, mortgage, student_loan, auto_loan,
             other_liability
  subtype text nullable
  currency default USD
  connection_type enum: manual, file_import, simplefin, teller, plaid, mx,
                        mastercard, other
  connector_item_id nullable
  credit_limit_minor nullable
  currency_exponent
  normal_balance enum: debit, credit
  cashflow_role enum: liquid_cash, credit_facility, loan_liability,
                      investment_asset, real_asset, external_clearing,
                      income_expense_virtual
  apr_bps nullable
  tags json
  is_retirement bool
  is_tax_advantaged bool
  is_joint bool
  is_business bool
  is_active bool
  last_synced_at nullable
  last_manual_balance_at nullable
  metadata_json
  created_at
  updated_at
```

Current and available balances are not canonical mutable account fields. They are exposed through `account_balance_read_model`, derived from ledger postings plus the latest accepted balance observations and reconciliation state.

```text
ledger_accounts
  id
  household_id
  user_account_id nullable
  category_id nullable
  name
  kind enum: asset, liability, income, expense, equity, clearing
  cashflow_role enum: liquid_cash, credit_liability, loan_liability,
                      investment_asset, real_asset, income, expense,
                      transfer_clearing, opening_balance, adjustment
  normal_balance enum: debit, credit
  currency nullable
  is_system bool
  is_active bool
  created_at
  updated_at

balance_observations
  id
  household_id
  account_id
  observed_at
  balance_date
  balance_type enum: ledger, available, statement, connector, manual
  amount_minor
  currency
  source_record_id nullable
  reconciliation_session_id nullable
  confidence_bps
  created_at
```

### 9.4 Ledger Transactions, Postings, and User-Facing Transactions

The canonical financial movement is stored as `ledger_transactions` plus balanced `ledger_postings`. User-facing transaction rows are read models, not canonical financial records. They are regenerated from ledger postings, categories, merchant identities, provenance, and review state.

```text
ledger_transactions
  id
  household_id
  source enum: manual, import, connector, document_extraction,
               generated_forecast_confirmation, adjustment, reversal
  source_record_id nullable
  transaction_date
  posted_date nullable
  authorized_date nullable
  description
  status enum: staged, pending, posted, voided, reversed
  reversal_of_id nullable
  provenance_id nullable
  audit_event_id
  created_at
  updated_at

ledger_postings
  id
  ledger_transaction_id
  ledger_account_id
  account_id nullable
  amount_minor
  currency
  currency_exponent
  direction enum: debit, credit
  category_id nullable
  merchant_identity_id nullable
  tags_json
  memo nullable
  is_user_adjusted bool
  created_at

transaction_display_rows_read_model
  id
  ledger_transaction_id
  display_account_id
  display_amount_minor
  currency
  original_description
  clean_description
  merchant_identity_id nullable
  primary_category_id nullable
  category_confidence_bps
  category_source enum: connector, system_rule, user_rule, local_model,
                        llm_suggestion, user_override, unknown
  notes
  is_pending bool
  recurring_event_id nullable
  review_status enum: none, needs_category, needs_transfer_review,
                      needs_recurring_review, needs_split_review,
                      needs_reconciliation_review
  created_at
  updated_at
  deleted_at nullable
```

Rules:

- No command writes directly to `transaction_display_rows_read_model`.
- User edits such as categorize, split, merge, reverse, amend date, or attach document are Finance Kernel commands.
- A successful edit updates canonical ledger/provenance/review tables and then refreshes the read model.
- Read-model rebuild must reproduce the same rows from canonical tables.

### 9.5 Transaction Splits

```text
split_groups
  id
  ledger_transaction_id
  display_name nullable
  created_by_command_id
  created_at

split_lines
  id
  split_group_id
  amount_minor
  currency
  currency_exponent
  category_id
  target_ledger_account_id
  tags_json
  notes
  created_at
  updated_at
```

Splits are user-facing editing conveniences over ledger postings. A split command creates or amends canonical postings while preserving source records, provenance, and audit trail. Saved split templates are separate from committed split groups.

Splits are critical for Amazon, Costco, Target, Venmo, PayPal, reimbursable work expenses, mixed personal/business purchases, and shared household purchases.

Ledger invariants:

- Postings for a committed ledger transaction must balance according to account normal balance rules.
- A transfer is not a special transaction type; it is a transaction with postings to two or more accounts.
- A credit card purchase increases a credit liability and expense/category postings, but does not reduce liquid cash until payment.
- A credit card payment reduces liquid cash and the card liability.
- User corrections create amendments or reversals with audit history.

### 9.6 Categories

```text
categories
  id
  household_id nullable
  parent_id nullable
  name
  type enum: income, expense, transfer, adjustment
  icon
  color
  is_system bool
  budget_default_minor nullable
  forecast_behavior enum: deterministic, variable_regular, variable_lumpy,
                          ignore_cashflow, income
  created_at
  updated_at
```

Default taxonomy should be useful but editable:

```text
Income
  Salary
  Hourly Wages
  Contractor/Freelance
  Bonus/Commission
  Rental Income
  Investment Income
  Benefits
Housing
  Rent/Mortgage
  Property Tax
  HOA
  Utilities
  Internet/Phone
  Insurance
  Maintenance
Food and Drink
  Groceries
  Restaurants
  Coffee
  Alcohol/Bars
Transportation
  Gas
  Auto Payment
  Auto Insurance
  Maintenance
  Parking/Tolls
  Public Transit
  Rideshare
Debt
  Credit Card Payment
  Student Loan
  Auto Loan
  Personal Loan
Health
  Insurance Premiums
  Medical
  Dental
  Pharmacy
Family
  Childcare
  Education
  Pet Care
Lifestyle
  Shopping
  Travel
  Entertainment
  Subscriptions
  Fitness
  Gifts/Donations
Savings and Investments
  Emergency Fund
  Brokerage Transfer
  Retirement Contribution
  HSA Contribution
Transfers
  Internal Transfer
  Reimbursement
  Cash Withdrawal
Taxes
  Federal Tax
  State Tax
  Local Tax
  Quarterly Estimated Tax
```

### 9.7 Income Sources and Events

```text
income_sources
  id
  household_id
  owner_profile_id nullable
  name
  type enum: w2_salary, w2_hourly, part_time, contractor_1099,
             freelance, rental_income, investment_income, benefits,
             other
  frequency enum: weekly, biweekly, semimonthly, monthly, quarterly,
                  annual, irregular, manual_schedule
  gross_amount_minor nullable
  net_amount_minor nullable
  hourly_rate_minor nullable
  currency
  currency_exponent
  expected_hours_per_period nullable
  pay_schedule_json
  tax_withholdings_json
  deductions_json
  confidence_json
  is_active
  created_at
  updated_at

income_events
  id
  income_source_id
  date
  gross_amount_minor nullable
  net_amount_minor
  hours_worked nullable
  withholdings_snapshot_json
  linked_transaction_id nullable
  source enum: manual, paystub, connector_match, forecast_confirmation
  created_at
```

### 9.8 Recurring Events

```text
recurring_events
  id
  household_id
  account_id nullable
  name
  description_pattern nullable
  amount_expected_minor
  amount_variance_minor nullable
  currency
  currency_exponent
  frequency enum: weekly, biweekly, semimonthly, monthly, quarterly,
                  annual, custom
  custom_schedule_json nullable
  next_expected_date
  category_id
  auto_detected bool
  detection_confidence_bps
  include_in_forecast bool
  autopay_account_id nullable
  is_active bool
  notes
  created_at
  updated_at

bill_contracts
  id
  household_id
  merchant_identity_id nullable
  name
  type enum: utility, rent_mortgage, insurance, subscription,
             loan_payment, tax, membership, childcare, other
  expected_amount_minor nullable
  amount_variance_minor nullable
  currency
  cadence enum: weekly, biweekly, semimonthly, monthly, quarterly,
                annual, custom, irregular
  due_rule_json
  autopay_enabled nullable
  autopay_account_id nullable
  payment_method_account_id nullable
  renewal_date nullable
  cancellation_url nullable
  contract_document_id nullable
  price_history_json
  status enum: active, paused, cancelled, suspected, needs_review
  include_in_forecast bool
  created_at
  updated_at

commitments
  id
  household_id
  commitment_type enum: rent, mortgage, utility, subscription, insurance,
                        loan_payment, credit_card_payment, tax_reserve,
                        childcare, tuition, membership, transfer, other
  name
  merchant_identity_id nullable
  amount_expected_minor nullable
  amount_confidence_bps
  due_rule_json
  payment_source_account_id nullable
  autopay_status enum: enabled, disabled, unknown
  source_entity_type nullable
  source_entity_id nullable
  include_in_forecast bool
  status enum: active, paused, cancelled, suspected, needs_review
  stale_after_date nullable
  created_at
  updated_at
```

Rules:

- `recurring_events` describe detected or configured recurrence patterns.
- `bill_contracts` describe merchant/contract metadata.
- `commitments` are the forecast-facing obligation records consumed by Future Cash and Money Inbox.

### 9.9 Credit Card Cycles

```text
credit_card_cycles
  id
  account_id
  statement_close_day nullable
  payment_due_day nullable
  grace_period_days nullable
  autopay_enabled nullable
  autopay_account_id nullable
  pay_behavior enum: pay_full_statement, pay_minimum, pay_fixed_amount,
                     pay_current_balance, unknown
  fixed_payment_minor nullable
  created_at
  updated_at

credit_card_statements
  id
  account_id
  cycle_start_date
  cycle_close_date
  due_date
  statement_balance_minor nullable
  minimum_payment_minor nullable
  paid_amount_minor
  forecast_statement_balance_minor nullable
  forecast_payment_minor nullable
  status enum: forecast, open, closed, paid, partial, late
```

### 9.10 Forecasts

```text
forecast_runs
  id
  household_id
  generated_at
  horizon_days
  starting_cash_minor
  model_version
  model_registry_json
  input_snapshot_id
  assumptions_hash
  assumptions_version
  scenario_overlay_ids_json
  random_seed nullable
  deterministic_run bool
  p10_min_cash_minor nullable
  p50_min_cash_minor nullable
  p90_min_cash_minor nullable
  summary_json

forecast_rows
  id
  forecast_run_id
  date
  description
  amount_p10_minor
  amount_p50_minor
  amount_p90_minor
  running_balance_p10_minor
  running_balance_p50_minor
  running_balance_p90_minor
  source_type enum: starting_balance, income, recurring_bill,
                    variable_spend, credit_card_payment,
                    transfer, scenario_event, manual_entry,
                    loan_payment, investment_cashflow
  source_id nullable
  confidence_bps
  explanation_json
  is_user_adjusted bool
  computation_mode enum: deterministic_incremental, full_batch
  stale_since nullable

forecast_input_snapshots
  id
  household_id
  created_at
  ledger_cutoff_at
  included_entity_hashes_json
  source_freshness_json
  schema_version

forecast_model_registry
  id
  model_name
  model_version
  feature_schema_version
  parameters_json
  created_at

forecast_backtest_results
  id
  forecast_run_id
  as_of_date
  horizon_days
  actual_min_cash_minor
  predicted_p10_min_cash_minor
  predicted_p50_min_cash_minor
  predicted_p90_min_cash_minor
  error_summary_json

forecast_actuals
  id
  forecast_row_id
  matched_entity_type nullable
  matched_entity_id nullable
  actual_date nullable
  actual_amount_minor nullable
  status enum: actualized_exact, actualized_matched, missed, superseded
  delta_minor nullable
  explanation_json
  created_at

forecast_quality_scores
  id
  forecast_run_id
  score_bps
  grade enum: high, medium, low, stale
  drivers_json
  created_at

forecast_assumption_events
  id
  household_id
  assumption_key
  assumption_type enum: income_amount, income_date, bill_amount, bill_date,
                        card_payment_behavior, minimum_cash_floor,
                        variable_spend_override, one_time_event,
                        inflation_rate, scenario_toggle, exclusion
  effective_from_date nullable
  effective_to_date nullable
  value_json
  supersedes_event_id nullable
  source enum: user, rule, import, connector, model, scenario
  confidence_bps
  created_by_command_id
  created_at

forecast_dependency_edges
  id
  forecast_run_id
  forecast_row_id
  dependency_type enum: ledger_posting, recurring_event, income_source,
                        bill_contract, balance_observation, assumption_event,
                        model_parameter, scenario_event
  dependency_id
  impact_minor nullable
  created_at

forecast_dirty_ranges
  id
  household_id
  from_date
  to_date
  reason
  caused_by_entity_type
  caused_by_entity_id
  queued_at
  resolved_by_forecast_run_id nullable
```

### 9.11 Agent Reports

```text
agent_runs
  id
  household_id
  agent_type enum: risk, planning, cash_runway, bill_audit, debt,
                   income_variability, spending_drift,
                   vacation_affordability, emergency_fund,
                   tax_reserve, document_extraction
  title
  summary
  full_output_markdown deprecated nullable
  output_json
  input_context_hash
  model_provider enum: local, openai, anthropic, other, none
  model_name
  tokens_input nullable
  tokens_output nullable
  cost_estimate_minor nullable
  status enum: draft, completed, failed, invalid_schema
  created_at
  is_starred bool
  tags_json

agent_evidence
  id
  agent_run_id
  entity_type
  entity_id
  quote_or_summary
  relevance

agent_report_blocks
  id
  agent_run_id
  block_order
  block_type enum: heading, paragraph, metric_card, evidence_list,
                   cashflow_table, risk_flag_ref, assumption_ref,
                   suggested_action, disclaimer
  content_json
  evidence_required bool
  created_at
```

### 9.12 Provenance and Source Records

```text
source_batches
  id
  household_id
  source_type enum: manual, csv, ofx, qfx, qif, pdf, image,
                    simplefin, teller, plaid, mx, relay, other
  source_name
  imported_at
  parser_version
  status enum: staged, partially_committed, committed, superseded, failed
  summary_json

source_records
  id
  source_batch_id
  external_id nullable
  source_hash
  raw_payload_ref nullable
  raw_payload_encrypted bool
  normalized_json
  parse_confidence_bps nullable
  created_at

provenance_links
  id
  entity_type
  entity_id
  source_record_id nullable
  audit_event_id nullable
  rule_id nullable
  agent_run_id nullable
  relationship enum: created_from, amended_by, inferred_from,
                     confirmed_by, contradicted_by, superseded_by
  created_at
```

---

## 10. Product Module Specifications

### 10.1 Secure Vault Module

Features:

- Create vault.
- Unlock vault.
- Lock vault.
- Change password.
- Rotate keys.
- Enable/disable Touch ID.
- Auto-lock after idle period, default 5 minutes.
- Lock on sleep/screensaver/user switch where feasible.
- Encrypted backup export.
- Encrypted backup restore.
- Read-only vault snapshot export for household partners.
- Redacted support export.
- Vault health check.

Acceptance criteria:

- Wrong password never opens vault.
- App restart requires unlock.
- Database file is unreadable with normal SQLite tools.
- Attachments are unreadable outside app.
- Logs remain safe after vault operations.
- WAL/SHM/temp-file behavior is tested for absence of plaintext financial data.
- Backup export and restore are tested with fixture vaults on every release.
- Recovery-key behavior, if enabled, has explicit tests for successful recovery and failed recovery attempts.

### 10.1.1 Read-Only Vault Snapshot

A read-only vault snapshot is an encrypted export of selected accounts, balances, forecasts, and reports that a household partner can open in their own app instance.

Snapshot properties:

- Encrypted with a separate snapshot password chosen at export time.
- Contains only the accounts, transactions, forecasts, and reports the primary user selects.
- Marked as read-only in vault metadata; the app disables all mutation commands when a snapshot vault is open.
- The snapshot has a generation timestamp and an expiration date (default: 30 days).
- Opening an expired snapshot shows a warning and suggests requesting a fresh export.
- Snapshot vaults use a distinct visual theme (e.g., subtle banner) to prevent confusion with the primary vault.
- Snapshot vaults cannot be upgraded to primary vaults.
- The primary user can regenerate a snapshot at any time with updated data.
- Snapshot export is logged in the primary vault's operation log.

Account filtering:

- The primary user selects which accounts to include in the snapshot.
- Individual transactions, documents, and agent reports can be excluded.
- The snapshot includes the forecast as generated, not the raw data to regenerate it.

This feature does not replace future household collaboration. It is a pragmatic intermediate for shared visibility without shared write access.

### 10.2 Onboarding Module

Progressive onboarding:

```text
Step 1: Create encrypted vault
  - Explain unrecoverable password risk.
  - Encourage encrypted backup.

Step 2: Choose primary goal
  [ ] Forecast my cash
  [ ] Track spending
  [ ] Manage credit cards/debt
  [ ] Track investments/net worth
  [ ] All of the above

Step 3: Add data
  [Add account manually]
  [Import file]
  [Connect account, optional]

Step 4: Set up income
  - Salary
  - Hourly
  - Contractor/lumpy
  - Skip for now

Step 5: Set up recurring bills
  - Add manually
  - Detect from imported transactions
  - Skip for now

Step 6: Review dashboard
  - Preconfigured based on goal
  - Show next recommended setup tasks
```

Principles:

- Do not overwhelm users with investments, agents, documents, taxes, and advanced forecasting all at once.
- Let power users jump to advanced setup.
- Every skipped step should be resumable from a setup checklist.

#### 10.2.1 First Forecast Wizard

The primary onboarding path should be a First Forecast Wizard optimized for time-to-first-useful-cash-projection.

Minimum required inputs:

1. household timezone and reporting currency
2. current liquid accounts and opening balances
3. next expected paycheck or income event
4. known recurring bills for the next 30 days
5. credit card due dates and expected payment behavior, optional but strongly recommended
6. minimum cash floor
7. one-time known upcoming expenses, optional

Output:

- first 30-day deterministic cash forecast
- forecast readiness score
- top 3 missing inputs by estimated forecast impact
- next recommended setup action

The wizard should not ask about investments, documents, agents, connectors, advanced categories, tax settings, or long-horizon planning.

### 10.3 Accounts Module

Core screens:

- Account list.
- Account detail.
- Balance history.
- Sync/import history.
- Connection health.
- Account tags.
- Include/exclude from cash forecasts.
- Retirement/non-retirement grouping.
- Joint/individual ownership.

Special handling:

- Cash accounts affect Future Cash.
- Credit cards affect statement/payment forecast, not liquid cash until paid.
- Investment accounts affect net worth, not Future Cash unless dividends/transfers/liquidations are modeled.
- Loans affect liabilities and scheduled payments.

### 10.4 Transactions Module

Features:

- Fast ledger with search/filter/sort.
- Bulk edit.
- Category editing.
- Split editing.
- Tags and notes.
- Attachment linking.
- Transfer matching.
- Recurring group linking.
- Review queue.
- Import/sync reconciliation.
- Undo for user edits.
- Quick-entry templates for fast manual transaction creation.

#### 10.4.1 Quick-Entry Templates

Quick-entry templates are pre-filled transaction forms that reduce repetitive data entry for manual-first users.

Template sources:

- **Auto-generated from recurring events:** Each active recurring event produces a template with the expected amount, account, category, and merchant pre-filled.
- **Auto-generated from recent transactions:** The last 5 unique merchant+category+account combinations are available as templates, with the most recent amount pre-filled.
- **User-saved templates:** Users can save any transaction as a reusable template with a custom name and optional default amount.
- **Split templates:** For ambiguous merchants (Costco, Amazon, Target), saved split configurations become templates.

Quick-entry flow:

```text
Cmd+N or "+" button
  -> Template autocomplete field (fuzzy search by merchant, category, or template name)
  -> Select template
  -> Pre-filled form: date (today), amount (editable), account, category, merchant
  -> User adjusts amount and/or date
  -> Enter to commit
```

Design rules:

- Templates are suggestions, not constraints. Every field is editable before commit.
- Template autocomplete searches across all template sources with unified ranking.
- The most-used templates appear first in the autocomplete list.
- Templates are stored in the vault and included in backup/export.
- Auto-generated templates do not require user configuration.
- Templates never auto-commit; the user always confirms before the transaction is created.

Ledger performance target:

- 100,000 transactions should remain usable.
- Virtualized table required.
- Queries indexed by account, posted date, amount, category, merchant identity, and review status.
- Ledger screens should query materialized read models, not reconstruct complex posting/category/provenance state on every render.
- Import, categorization, merchant normalization, recurring detection, and forecast refresh should run as cancelable background jobs.

### 10.5 Documents Module

Document types:

- Pay stubs.
- Bank statements.
- Credit card statements.
- Brokerage statements.
- Bills.
- Receipts.
- Tax documents.
- Insurance documents.

Document flow:

1. User imports PDF/image/file.
2. File is encrypted immediately.
3. Metadata is stored.
4. Optional extraction runs locally or through explicit BYOK/cloud flow.
5. Extracted fields enter a review state.
6. User confirms or corrects.
7. Confirmed data can update income sources, bills, accounts, or transactions.

Rules:

- Raw OCR text is sensitive.
- Extraction outputs must be editable.
- Do not silently change financial assumptions from a document extraction.

---

## 11. Transaction Categorization and Learning

### 11.1 Categorization Philosophy

Categorization cannot be a simple merchant-to-category map. That fails for Amazon, Venmo, PayPal, eBay, Target, Costco, Apple, Square, Stripe, Zelle, cash withdrawals, and card processors.

Use a layered system:

```text
Layer 0: Connector/import category, treated as weak signal
Layer 1: Merchant normalization
Layer 2: Known recurring event matching
Layer 3: User-defined rules
Layer 4: Feature-based local classifier
Layer 5: Contextual disambiguation
Layer 6: Optional LLM batch suggestion
Layer 7: Confidence threshold + review queue
Layer 8: Learning from overrides and splits
```

### 11.2 Features for Categorization

| Feature | Example use |
|---|---|
| Clean merchant | Safeway usually groceries |
| Raw description | `SQ *COFFEE SHOP` reveals Square processor |
| Amount | Amazon $14.99 monthly may be subscription; $348 may be shopping |
| Day/time | Venmo Friday night vs rent on 1st |
| Account | Business card vs personal debit |
| MCC | Useful when provided |
| Historical behavior | Similar transactions by user |
| Recurrence | Stable amount/date implies subscription/bill |
| Nearby transactions | Gas + hotel + airline implies travel context |
| Notes/tags | User-provided context |
| Receipt/document match | Confirms split or category |
| Split history | Costco split into groceries/household/other |

### 11.3 User Rules

Rules should support:

- Merchant contains.
- Raw description regex.
- Amount range.
- Account.
- Date/day conditions.
- Recurrence pattern.
- Category assignment.
- Tag assignment.
- Split template.
- Priority order.
- Test rule against historical transactions before applying.

### 11.4 Local Classifier

Start with simple local models:

- Logistic regression, gradient-boosted trees, or Naive Bayes over engineered features.
- Train per household.
- Do not ship user data to train a global model without explicit opt-in.
- Retrain after N corrections or on a schedule.
- Keep model version and feature schema version.

### 11.5 Confidence and Review

Every automatic category needs:

```text
category
confidence
source
reason
model_version
features_used_summary
```

Rules:

- High confidence: auto-apply.
- Medium confidence: apply but mark reviewable.
- Low confidence: queue for review.
- User override becomes a training signal.
- User override does not become a universal hard rule unless user chooses.

### 11.6 Ambiguous Merchant Handling

Examples:

- **Venmo/Zelle:** Look at amount, date, notes if available, counterparty, recurrence, and historical matching.
- **Amazon:** Look for monthly amount, known subscription amount, receipt import, split behavior, and category distribution history.
- **Costco/Target/Walmart:** Encourage split templates and receipt matching.
- **Apple:** Separate iCloud, App Store, hardware, subscriptions, Apple Card payments.
- **Square/Stripe/PayPal:** Treat processor as weak merchant; infer from raw string and user history.

---

## 12. Income Modeling

### 12.1 Income Source Types

| Type | Forecasting method |
|---|---|
| W-2 salary | Pay schedule + gross/net model + confirmed paycheck history |
| W-2 hourly | Expected hours x rate + historical variance + user-entered upcoming hours |
| Part-time | Similar to hourly, wider variance by default |
| Contractor/freelance | Manual expected invoices + historical cadence + scenario bands |
| Commission/bonus | Scenario/event-based with probability |
| Rental income | Lease schedule + vacancy/late-payment risk later |
| Investment income | Dividends/interest from holdings or manual schedule |
| Benefits/government | Fixed schedule |
| Irregular support/gifts | Manual scenario entries |

### 12.2 Salary/W-2 Flow

Inputs:

- Annual salary.
- Pay frequency.
- Next pay date or anchor date.
- Filing status.
- Federal/state/local withholding assumptions.
- Pre-tax deductions.
- Post-tax deductions.
- Retirement contribution.
- HSA/FSA.
- Health insurance.
- Employer match, for net worth/retirement tracking.

Outputs:

- Forecast net paycheck.
- Pay dates.
- Confidence.
- Difference from actual paycheck once matched.

Implementation note:

- Build a conservative payroll estimator, but avoid representing it as tax advice.
- Support manual net paycheck override because payroll systems and withholdings vary.

### 12.3 Hourly/Semi-Recurring Flow

Inputs:

- Hourly rate.
- Typical hours/week.
- Pay frequency.
- Upcoming scheduled hours.
- Historical paycheck events.

Forecast:

- Use user-entered upcoming hours when present.
- Use rolling historical average when not present.
- Add uncertainty bands from historical variance.
- Bayesian updating: begin with user estimate, update as actual paychecks arrive.

### 12.4 Contractor/Lumpy Income Flow

Inputs:

- Expected invoices.
- Expected payment date.
- Probability or confidence.
- Historical payments.
- Client tags.
- Tax reserve percentage.

MVP contractor mode:

- User may set a self-chosen reserve percentage for contractor income.
- Reserved cash is modeled as unavailable-to-spend in Future Cash.
- The app labels this as a planning reserve, not tax advice.
- Reserve transfers can be suggested as manual actions but never initiated.
- Tax Reserve Estimator remains a later agent/report feature.

Forecast:

- Deterministic expected invoices in P50 if high confidence.
- Wider P10/P90 bands.
- Optional runway analysis excluding unconfirmed invoices.
- Quarterly tax reserve warning if enabled.

### 12.5 Pay Stub Parser

Extract:

- Employer.
- Pay period.
- Pay date.
- Gross pay.
- Net pay.
- Hours.
- Federal withholding.
- State withholding.
- Local withholding.
- Social Security.
- Medicare.
- Pre-tax deductions.
- Post-tax deductions.
- Retirement contribution.
- Employer match if visible.
- HSA/FSA.
- Health insurance.
- Garnishments/other deductions.

Rules:

- Always require user review before updating income model.
- Preserve original pay stub encrypted.
- Store extraction confidence per field.
- Allow manual corrections.

---

## 13. Future Cash Forecasting Engine

### 13.1 Forecasting Principle

The Future Cash engine is a forward-looking daily cash ledger. It combines:

1. Deterministic known events.
2. Detected recurring events.
3. Statistical variable spending.
4. Credit card statement/payment modeling.
5. Income uncertainty.
6. Scenario overlays.
7. Risk detection.

It should not use simple linear extrapolation.

It should also be reproducible. A full forecast run must be regenerable from:

- input data snapshot ID
- model registry versions
- assumptions version
- scenario overlay IDs
- calendar version
- random seed, when stochastic simulation is used
- code/model version

This is required for trust, backtesting, debugging, and user-visible explanations.

Forecast rows are outputs, not editable source records. Any user edit in Future Cash creates an immutable forecast assumption event. Forecast generation consumes the latest active assumption state plus source data snapshots.

### 13.1.1 Two-Tier Forecast Architecture

The forecast engine operates in two modes:

**Interactive mode (deterministic incremental):**

- Used when the user edits a single assumption, amount, date, or manual entry.
- Propagates the change forward from the affected date through the deterministic ledger only.
- Recalculates running balances from the first affected row to the end of the horizon.
- Does not re-run Monte Carlo simulation or stochastic spending models.
- Confidence bands are marked "approximate — last full run: [timestamp]" until the next batch run.
- Target latency: under 50 ms for a single-assumption change on a 1-year horizon.
- The deterministic layer is a pure function: `f(sorted_events, starting_balance) -> daily_balances`.

**Batch mode (full regeneration):**

- Used for official forecast snapshots, backtests, simulation runs, and periodic refresh.
- Generates complete `forecast_run` and `forecast_rows` records with full provenance.
- Runs Monte Carlo simulation for variable and lumpy spending.
- Computes P10/P50/P90 bands, risk flags, and quality scores.
- Runs as a background job; UI updates asynchronously when complete.
- Triggered by: app launch, significant data change (import, connector sync, reconciliation), user-requested refresh, or periodic schedule (e.g., daily).

**Interaction between modes:**

- Interactive edits are immediately reflected in the deterministic ledger view.
- Interactive edits write forecast assumption events and mark forecast dirty ranges. They do not directly mutate generated forecast rows.
- A background batch run is automatically queued after interactive edits settle (debounced, e.g., 5 seconds of inactivity).
- When the batch run completes, the UI silently updates confidence bands and risk flags.
- The user never waits for a batch run to see the effect of their edit on the deterministic forecast.
- Forecast rows carry a `computation_mode` flag: `deterministic_incremental` or `full_batch`.

### 13.2 Inputs

| Input | Source |
|---|---|
| Current cash balances | Manual balances, imports, connectors |
| Income schedules | Income sources and paystub confirmations |
| Recurring bills | Manual setup and detection |
| Credit card cycles | Account settings and statements |
| Variable spending history | Categorized transactions |
| Transfers | User rules and transfer matching |
| Loan schedules | Loan module |
| Scenarios | User-created events |
| Manual forecast entries | Future Cash UI |
| Model assumptions | Forecast settings |
| Inflation rates | Optional per-category or household-wide annual rate, applied at 90+ day horizons |

### 13.2.1 Cash Availability Model

Future Cash should not begin from a single ambiguous balance. It needs a cash availability model that distinguishes:

- ledger balance
- available balance
- pending transactions
- uncleared deposits
- committed obligations
- card statement liabilities
- autopay source account
- manually reserved funds
- user-defined tax reserve allocations
- minimum cash floor

The UI can still show one simple number, but the forecast engine should retain the components. This prevents common user-confusing errors such as treating credit card purchases as immediate cash outflow, treating pending deposits as guaranteed cash, or double-counting autopay.

### 13.2.2 Forecast Actualization and Quality Score

Every forecast row should eventually become one of four states:

```text
actualized_exact | actualized_matched | missed | superseded
```

Add a forecast quality score that explains whether the current projection is trustworthy. Inputs:

- starting balance freshness
- reconciliation status
- percentage of upcoming obligations confirmed
- income confirmation status
- connector/import freshness
- forecast backtest error for similar horizons
- number and severity of unresolved review items

This score should appear in Future Cash and Dashboard. It is more honest than showing precise P10/P50/P90 numbers when the underlying data is stale.

Rename the user-facing form of this score to `Forecast Readiness`.

Forecast Readiness examples:

- "High: balances updated today; next paycheck confirmed; 92% of upcoming obligations confirmed."
- "Medium: credit card payment behavior unknown; checking balance is 5 days old."
- "Low: starting balance missing; recurring bills incomplete."

Every readiness factor should link to the action that improves it.

### 13.3 Three-Layer Projection Model

#### Layer 1: Deterministic Ledger

High-confidence events:

- Known paychecks.
- Rent/mortgage.
- Subscriptions.
- Insurance.
- Loan payments.
- Scheduled transfers.
- Known card payments.
- Manually entered one-time events.

Implementation note: The deterministic ledger is the core of interactive mode. It must be implementable as a single-pass forward scan over a sorted event list. No database queries, no stochastic sampling, and no network calls should occur in this path. This enables sub-50ms recalculation for interactive edits.

#### Layer 2: Statistical Variable Spending

Medium-confidence estimates:

- Groceries.
- Restaurants.
- Shopping.
- Gas.
- Entertainment.
- Healthcare.
- Travel.
- Household supplies.

Methods:

- Rolling averages weighted toward recent behavior.
- Exponential smoothing.
- Holt-Winters where sufficient seasonality exists.
- Day-of-week and day-of-month patterns.
- Category-specific variance.
- Fallback models for sparse data.

#### Layer 3: Behavioral/Trend Adjustment

Trend detection:

- Spending drift.
- Category substitution, such as restaurants replacing groceries.
- Vacation seasonality.
- Large recurring annual expenses.
- New subscription clusters.
- Income volatility shifts.

Methods:

- Change-point detection such as PELT or Bayesian change-point methods.
- Recency-weighted reforecasting after detected shifts.
- Risk flag generation when forecast impact is meaningful.

### 13.3.1 Inflation and Long-Horizon Adjustment

At horizons beyond 90 days, variable spending estimates should optionally incorporate inflation.

Design:

- Default household inflation rate: 0% (no adjustment unless configured).
- User may set a single household-wide rate or per-category overrides.
- Inflation compounds monthly on variable spending categories only; fixed contracts use their stated amounts.
- Bill contracts with known price escalation clauses use those instead of the general rate.
- The forecast explanation for each affected row shows the inflation adjustment.
- Inflation rates are stored as forecast assumptions and versioned.
- Backtesting should measure whether inflation-adjusted forecasts reduce long-horizon error.

This is explicitly not a macroeconomic model. It is a user-controlled drift parameter that prevents
long-horizon forecasts from systematically underestimating future spending.

#### Layer 4: Simulation and Calibration

Use deterministic ledgers for known events and stochastic distributions for uncertain events.

Recommended approach:

- Generate deterministic base cash ledger.
- Generate uncertain income/spend distributions by category/source.
- Run Monte Carlo simulations for variable and lumpy cashflows.
- Derive P10/P50/P90 cash balance bands from simulated daily balances.
- Attribute risk flags to the sources that most affect downside outcomes.
- Calibrate model confidence with historical backtests.

### 13.4 Lumpy Spending Categories

Travel, medical, car repairs, gifts, taxes, home maintenance, and major purchases need special handling.

Recommended model:

```text
probability of occurrence over horizon
x distribution of amount if occurrence happens
x seasonal/calendar adjustment
x user-specific history
x known scenario events
```

Example:

- User historically takes 2-3 major trips per year.
- No trip spend appears yet this year.
- Shopping is above trend.
- Forecast should not assume $0 travel forever.
- Risk Agent should flag that normal travel behavior may be unaffordable unless spending changes.

### 13.5 Credit Card Statement and Payment Modeling

This is critical because credit cards separate spending date from cash impact.

For each credit card:

1. Determine current cycle start/close.
2. Estimate remaining cycle spending by category.
3. Forecast statement balance.
4. Determine payment behavior.
5. Forecast payment date.
6. Model cash impact on payment date.
7. If not paid in full, estimate interest and payoff trajectory.

Card payment behavior options:

- Pay full statement balance.
- Pay current balance.
- Pay minimum.
- Pay fixed amount.
- Unknown, infer from history.

Risk flags:

- Statement balance projected above usual surplus.
- Utilization approaching threshold.
- Payment due before next paycheck.
- Pay-in-full behavior at risk.
- Revolving balance interest increasing.

### 13.6 Forecast Output Schema

For every future date with relevant events:

```text
date
starting_cash
known_income
expected_income
known_expenses
expected_expenses
credit_card_payment_effect
transfers
scenario_events
ending_cash_p10
ending_cash_p50
ending_cash_p90
confidence
risk_flags
explanation
```

### 13.7 Scenario Semantics

Use clear percentile language:

- **P10 cash balance:** pessimistic/lower cash outcome.
- **P50 cash balance:** expected/median outcome.
- **P90 cash balance:** optimistic/higher cash outcome.

For expenses, P90 expense means higher spending. To avoid confusion, UI should label cash outcomes rather than raw expense percentiles.

### 13.7.1 Safe-to-Spend Derivation

Safe-to-spend is a derived metric, not a stored value. It represents the maximum
discretionary spending available today without causing the P10 (pessimistic) cash
balance to drop below a user-defined floor at any point in the forecast horizon.

Calculation:

```text
safe_to_spend = min(
  current_liquid_cash - minimum_cash_floor,
  min_over_horizon(forecasted_p10_balance) - minimum_cash_floor
)
```

Rules:

- The minimum cash floor defaults to $0 but is user-configurable (e.g., $1,000 buffer).
- Safe-to-spend is computed from the P10 (pessimistic) forecast, not the P50 median.
- The horizon for the calculation defaults to the next full billing cycle (typically 30 days).
- If any input to the calculation is stale (balance older than 3 days, missing income
  confirmation), the safe-to-spend number displays with a "stale data" warning.
- If Forecast Readiness is below the configured threshold, safe-to-spend is hidden by default and replaced with: "Forecast inputs are incomplete. Review these items before relying on this number."
- If safe-to-spend is negative, the app shows "Upcoming obligations exceed available cash"
  with a link to the risk flags.
- Safe-to-spend is never presented as a spending recommendation.
- The UI must avoid advice-like copy such as "you can afford this" or "you should spend." Preferred language:
  - "Based on current assumptions..."
  - "This projection estimates..."
  - "This number excludes..."
  - "Review these assumptions before relying on this."
- The assumptions behind the number are one click away.
- Users can disable safe-to-spend entirely.

### 13.8 Future Cash UI

```text
+--------------------------------------------------------------------------------+
| Future Cash                                      1W 1M 3M 6M 1Y   Scenario: Base |
+--------------------------------------+-----------------------------------------+
| Ledger                               | Cash Projection                         |
|                                      |                                         |
| Today      Starting cash     $24,350 |  Line chart: P50 expected cash          |
| May 03     Grocery estimate    -$145 |  Band: pessimistic to optimistic        |
| May 05     Electric bill       -$109 |  Markers: bills, paydays, risk zones    |
| May 09     Paycheck          +$4,820 |                                         |
| May 15     Mortgage          -$3,100 |  Minimum cash threshold line            |
| May 18     Card payment est. -$2,450 |                                         |
| May 24     Utilities           -$385 |  Click a marker to inspect assumption   |
| May 31     Expected cash      $22,615|                                         |
|                                      |                                         |
| Risk flags                           |                                         |
| - Dining spend trending up           |                                         |
| - July card payment may exceed surplus|                                        |
+--------------------------------------+-----------------------------------------+
| + Add entry   Manage recurring   Edit assumptions   Backtest   Export           |
+--------------------------------------------------------------------------------+
```

### 13.8.1 Financial Calendar View

The calendar is a read-only projection of the same forecast data shown in the ledger and chart views. It answers the question: "Which specific days are tight?"

```text
+---------------------------------------------------------------------------+
| Financial Calendar                              May 2026        < Today > |
+-----------+-----------+-----------+-----------+-----------+-----------+----+
| Mon       | Tue       | Wed       | Thu       | Fri       | Sat       | Su |
+-----------+-----------+-----------+-----------+-----------+-----------+----+
|         1 |         2 |         3 |         4 |         5 |         6 |  7 |
| $24,350   |           | Grocery   | Electric  |           |           |    |
|           |           |  -$145    |  -$109    |           |           |    |
| [green]   | [green]   | [green]   | [green]   | [green]   | [green]   |    |
+-----------+-----------+-----------+-----------+-----------+-----------+----+
|         8 |         9 |        10 |        11 |        12 |        13 | 14 |
|           | Paycheck  |           |           |           |           |    |
|           | +$4,820   |           |           |           |           |    |
| [green]   | [green]   | [green]   | [green]   | [green]   | [green]   |    |
+-----------+-----------+-----------+-----------+-----------+-----------+----+
|        15 |        16 |        17 |        18 |        19 |        20 | 21 |
| Mortgage  |           |           | Card pmt  |           |           |    |
| -$3,100   |           |           | -$2,450   |           |           |    |
| [yellow]  | [yellow]  | [yellow]  | [yellow]  | [green]   | [green]   |    |
+-----------+-----------+-----------+-----------+-----------+-----------+----+
```

Design rules:

- Cell background color reflects ending P10 balance proximity to minimum floor.
- Green: P10 balance > 2x minimum floor.
- Yellow: P10 balance between 1x and 2x minimum floor.
- Red: P10 balance below minimum floor.
- Multiple events per day are stacked with truncation and expand on click.
- Dragging a manual forecast entry to a different day updates its date.
- The calendar does not introduce new data; it queries the same read model as the ledger.

### 13.9 Risk Flag Schema

```text
risk_flags
  id
  forecast_run_id
  severity enum: info, watch, warning, critical
  type enum: spending_trend, income_risk, balance_risk,
             savings_depletion, credit_utilization,
             goal_at_risk, stale_data, upcoming_lumpy_expense,
             tax_reserve_risk, debt_interest_risk
  title
  description
  projected_impact_minor
  time_horizon_days
  evidence_json
  suggested_actions_json
  readiness_required_bps
  suppressed_reason nullable
  created_at
```

Example:

```json
{
  "severity": "warning",
  "type": "spending_trend",
  "title": "Dining spend is trending up",
  "description": "Restaurant spending has increased from $420/month to $567/month over the last 3 months. If this continues, the July card payment is projected to exceed normal monthly surplus.",
  "projected_impact_minor": -44100,
  "time_horizon_days": 90,
  "suggested_actions": [
    "Set dining target to $450/month for the next 60 days",
    "Review recent restaurant transactions for subscriptions or delivery habits"
  ]
}
```

### 13.10 Forecast Backtesting

Backtesting is required for trust.

Approach:

1. Pick historical date T.
2. Hide transactions after T.
3. Generate forecast from data available at T.
4. Compare forecast to actual outcomes.
5. Store forecast error by horizon/category/model version.
6. Surface model confidence improvements over time.

Metrics:

- 7-day cash forecast error.
- 30-day cash forecast error.
- 90-day cash forecast error.
- Category spending error.
- Income forecast error.
- Credit card statement forecast error.
- Recurring detection precision/recall.

---

## 14. Budgeting and Goals

Budgeting should be tied to the forecast, not isolated from it.

### 14.1 Budget Types

- Category monthly budget.
- Envelope-style allocation, optional later.
- Rolling average target.
- Hard cap target.
- Savings-rate target.
- Goal funding schedule.
- Debt payoff schedule.

### 14.2 Goals

Goal examples:

- Emergency fund.
- Vacation.
- Mortgage/down payment.
- Baby/family planning.
- Job transition runway.
- Car purchase.
- Debt payoff.
- Annual tax reserve.
- Retirement contribution target.

Each goal should have:

```text
target_amount
target_date
current_progress
funding_source_accounts
monthly_required_contribution
forecast_status
risk_flags
scenario_links
```

### 14.3 Budget-to-Forecast Reconciliation

The app should answer:

- Are we under/over budget this month?
- Does being over budget actually threaten future cash?
- Which future date becomes risky if this continues?
- What adjustment restores the desired path?

---

## 15. Spending, Credit, and Debt Management

### 15.1 Credit Card View

Show:

- Current balance.
- Pending balance.
- Available credit.
- Credit limit.
- Utilization.
- Statement close date.
- Payment due date.
- Forecasted statement balance.
- Forecasted payment amount.
- Autopay account.
- Interest risk.
- Category spend on this card.

### 15.2 Debt Tools

Support:

- Minimum payment schedules.
- APR.
- Snowball payoff.
- Avalanche payoff.
- Fixed extra payment scenario.
- Refinance scenario, later.
- Debt-free date.
- Total interest estimate.

### 15.3 Interest Accrual

For revolving balances:

- Track APR.
- Estimate daily periodic rate.
- Estimate interest when full payment is not expected.
- Show uncertainty and assumptions.

Do not overcomplicate early; start with conservative estimates and explicit assumptions.

---

## 16. Investments and Net Worth

### 16.1 Product Boundary

Investments matter for net worth, allocation, and planning. They should not automatically affect cash forecasts unless the user models a dividend, sale, liquidation, or transfer.

### 16.2 Features

- Account-level balances.
- Holding-level positions.
- Quantity, price, value.
- Cost basis where available.
- Asset class mapping.
- Retirement vs taxable grouping.
- Traditional vs Roth tags.
- HSA tags.
- Crypto assets.
- Real estate/manual property.
- Alternative investments.
- Manual valuation snapshots.
- Net worth timeline.

### 16.3 Return Calculations

Start with:

- Simple total return.
- Contribution-adjusted snapshots.

Later:

- Time-weighted return.
- Money-weighted return/internal rate of return.
- Tax lot/cost basis details.

### 16.4 Allocation

Default allocation buckets:

- U.S. equity.
- International equity.
- Bonds.
- Cash equivalents.
- Real estate.
- Crypto.
- Alternatives.
- Unknown.

The user should be able to override mappings.

---

## 17. AI Agent System

### 17.1 Insight Engine Before Agent Library

Before shipping many named agents, build an **Insight Engine** that turns deterministic findings into user-facing cards and optional reports.

Insight types:

- forecast moved materially
- starting balance stale
- upcoming obligation unconfirmed
- category drift detected
- card payment risk
- duplicate subscription suspected
- income variance changed
- backtest error worsened

Each insight has severity, evidence, affected date, estimated cash impact, suggested next action, and dismissal/snooze state. LLM narration can later summarize these cards, but the deterministic insight record is the product primitive.

### 17.2 Agent Philosophy

Agents are not chatbots. They are narrowly scoped report generators.

Most agents should be two-stage systems:

1. deterministic analyzer computes findings, metrics, evidence, and proposed actions
2. optional LLM narrator converts the structured findings into a readable report

The LLM should not be responsible for core arithmetic, ledger interpretation, forecast generation, or data reconciliation.

Agents should:

- Have fixed purposes.
- Use explicit allowed data views.
- Produce schema-validated outputs.
- Produce typed report blocks that are rendered by a safe local renderer.
- Cite local evidence.
- Respect token/cost limits.
- Be saved in an agent library.
- Be reproducible from input context hashes where possible.

Agents should not:

- Execute arbitrary tools.
- Browse the web by default.
- Make payments.
- Move money.
- Trade assets.
- Edit financial data.
- Store hidden instructions from user documents.
- Recompute financial truth from raw text when deterministic records are available.
- Produce uncited recommendations.
- Render arbitrary Markdown, HTML, SVG, remote images, scripts, or model-supplied links.
- Convert suggestions into committed actions.

Agent outputs should separate:

- findings
- evidence
- assumptions
- uncertainty
- proposed actions
- user-confirmed actions

### 17.3 Agent Broker Pattern

```text
User selects agent/report type
  -> Rust Agent Broker validates request
  -> Broker runs deterministic analyzer when available
  -> Broker gathers minimal allowed local data views
  -> Broker redacts/minimizes fields based on agent policy
  -> Broker builds structured findings capsule
  -> Optional cost preview and consent
  -> Model call, local or BYOK cloud
  -> Response schema validation
  -> Safety checks
  -> Store encrypted report and evidence citations
  -> Render report in UI
```

### 17.4 Agent Definition Schema

```text
agent_definition
  name
  purpose
  allowed_data_views
  forbidden_data
  allowed_tools
  model_modes_allowed: local, byok_cloud, managed_cloud
  max_input_tokens
  max_output_tokens
  max_estimated_cost_minor
  temperature
  output_schema
  evidence_requirements
  user_confirmation_required
  disclaimer
  deterministic_analyzer nullable
  pii_budget
  output_action_schema
  max_evidence_items
```

### 17.5 Initial Agents

| Agent | Deliverable | Notes |
|---|---|---|
| Risk Agent | Household financial risk report | Spending drift, balance risk, stale data, credit risk |
| Cash Runway Agent | Cash shortfall and runway analysis | Especially useful for contractors/job changes |
| Planning Agent | Scenario impact report | Mortgage, baby, job change, relocation, car purchase |
| Bill/Subscription Agent | Recurring bill audit | Detects duplicate subscriptions, annual renewals |

The Bill/Subscription Agent should operate on `bill_contracts`, recurring events, transaction history, and documents. It should not be the primary source of recurring-obligation truth.
| Debt Agent | Payoff plan | Snowball/avalanche, interest impact |
| Income Variability Agent | Income risk report | Hourly/contractor uncertainty |
| Spending Drift Agent | Lifestyle creep analysis | Category substitution and trend change |
| Vacation Affordability Agent | Travel feasibility report | Uses historical travel behavior and future cash |
| Emergency Fund Agent | Emergency fund adequacy | Uses expenses, obligations, income volatility |
| Document Agent | Structured extraction report | Paystub/statement extraction with review |
| Tax Reserve Estimator | Estimated tax reserve report | Later; careful disclaimers and settings required |

### 17.6 Cost Controls

- BYOK keys stored only with user permission.
- Per-agent hard token caps.
- Daily/monthly usage limits.
- Cost estimate before cloud call.
- Usage dashboard.
- No hidden background agent runs unless explicitly enabled.
- Rate limits for automatic reports.
- Managed premium, if ever added, should use quotas rather than unlimited use.

### 17.7 Prompt Injection Controls

- Imported text is data, not instruction.
- System prompts are compiled or signed, not user-editable config.
- User input is parameterized for scenario agents.
- Agent tools are static and read-only.
- LLM output is schema-validated.
- Reports include evidence citations to local records.
- Never pass provider tokens, account numbers, raw keys, or unnecessary PII to models.
- Provide local-only mode.

---

## 18. UX and Information Architecture

### 18.1 Main Navigation

```text
Dashboard
Future Cash
Financial Calendar
Transactions
Accounts
Income
Credit & Debt
Investments
Planning
Documents
Agent Reports
Settings
```

For MVP, hide advanced sections until configured.

### 18.1.1 Money Inbox

Add a single triage surface called **Money Inbox**. It collects all items that need attention before they pollute core screens.

Inbox item types:

- imported transactions waiting for commit
- low-confidence categories
- possible transfers
- possible recurring bills
- stale balances
- document extractions needing confirmation
- forecast assumptions needing attention
- connector errors
- reconciliation discrepancies

Money Inbox prevents the dashboard from becoming a noisy alert wall and gives the user one keyboard-friendly place to restore data quality.

### 18.2 Dashboard

Default widgets:

- Liquid cash today.
- Future cash 30-day mini-chart.
- Upcoming bills.
- Upcoming income.
- Next card payment estimate.
- Risk flags.
- Transactions needing review.
- Connection health.
- What changed since last open.
- Decisions needing attention.
- Forecast drivers.
- Safe-to-spend guardrail, optional and assumption-based.

Optional widgets:

- Net worth.
- Investment allocation.
- Debt payoff progress.
- Goal progress.
- Spending by category.
- Income variability.
- Budget progress.
- Recent documents.
- Agent report summaries.

### 18.2.1 Daily Check-In Workflow

The daily check-in should answer:

- What changed since the last app open?
- Did any account balance, bill, income event, category trend, or card payment materially change the forecast?
- Which future date is now most constrained?
- What assumptions are stale?
- Which review items most improve forecast quality?
- What actions could restore the target path?

Action examples:

- update stale balance
- confirm upcoming paycheck
- review suspected duplicate subscription
- lower discretionary target for the next 30 days
- add expected one-time expense
- reconcile statement period

Actions are suggestions only. The app must not initiate payments, transfers, trades, or destructive edits.

### 18.2.2 Change Journal Subsystem

The Change Journal is a materialized projection that records user-meaningful state changes.

Each journal entry contains:

```text
change_journal_entries
  id
  household_id
  occurred_at
  change_type enum: balance_update, new_transactions, forecast_shift,
                    risk_flag_created, risk_flag_resolved, connector_status,
                    reconciliation_completed, category_drift, income_event,
                    bill_change, goal_progress, scenario_impact,
                    stale_data_warning, import_completed
  entity_type
  entity_id
  summary_text
  forecast_impact_minor nullable
  severity enum: info, notable, important
  seen_at nullable
  created_at
```

Rules:

- Journal entries are generated as a side-effect of event processing, not by polling.
- The daily check-in UI queries unseen journal entries ordered by severity and forecast impact.
- Entries older than 90 days are compacted into weekly summaries.
- Journal entries reference source events for drill-down.
- The "forecast impact" field enables ranking changes by how much they moved the cash outlook.

### 18.3 Dashboard Profiles

Support saved dashboard layouts:

- Daily check-in.
- Monthly review.
- Debt payoff.
- Contractor runway.
- Investment overview.
- Household admin.

### 18.4 Progressive Disclosure

Guidelines:

- Show a simple default path.
- Put advanced settings behind "Advanced" sections.
- Use review queues rather than modal interruptions.
- Give users quick wins during onboarding.
- Make every auto-detected item user-editable.

### 18.4.1 Keyboard-First Interaction Model

The product targets technical, privacy-conscious users who prefer keyboard interaction.
Keyboard-first is a design principle, not just an accessibility checkbox.

Core keyboard features:

- **Command palette** (Cmd+K): global fuzzy search over actions, navigation targets,
  accounts, recent transactions, templates, and settings. This is the primary power-user entry point.
- **Quick entry** (Cmd+N): open a template-aware transaction entry form from anywhere in the app.
  Typing in the quick-entry field fuzzy-matches against saved templates, recent merchants,
  and recurring events. Selecting a match pre-fills the form.
- **Quick search** (Cmd+F or /): focus the transaction/entity search field.
- **Navigation shortcuts**: Cmd+1 through Cmd+9 for main nav sections.
- **Transaction ledger navigation**: j/k for row movement, Enter to expand,
  e to edit, c to categorize, s to split, t to tag, Tab to move between fields.
- **Review queue shortcuts**: a to accept suggestion, r to reject, n for next item.
- **Forecast interaction**: arrow keys to navigate forecast rows, Enter to inspect assumptions.
- **Bulk operations**: Shift+click or Shift+j/k for multi-select, then bulk action.
- **Escape**: close modals, cancel edits, return to previous context.

Design rules:

- Every action reachable by mouse must also be reachable by keyboard.
- Shortcut hints should appear on hover and in the command palette.
- Shortcuts must not conflict with system or Tauri shortcuts.
- Keyboard shortcuts are documented in-app and customizable later.

### 18.5 Review Queues

Queues:

- Uncategorized transactions.
- Low-confidence categories.
- Possible recurring bills.
- Possible transfers.
- Stale balances.
- Document extractions needing confirmation.
- Forecast assumptions needing attention.
- Connector errors.

### 18.6 Accessibility

- Keyboard navigation.
- Screen-reader labels.
- High contrast support.
- Reduced motion.
- Color-blind-safe chart encoding.
- Table shortcuts.
- Exportable reports.

---

## 19. Reliability, Sync, and Data Quality

### 19.1 Reliability Goals

- Manual mode never breaks because of external provider issues.
- Forecasting continues with stale data, clearly labeled.
- Sync failures are visible and actionable.
- Import/sync dedupe prevents duplicate transaction chaos.
- Migrations are reversible or backup-protected.

### 19.2 Sync Design

For each connector item:

```text
connector_item
  provider
  status
  last_successful_sync_at
  last_attempted_sync_at
  next_suggested_sync_at
  cursor
  error_code
  error_message_redacted
  reauth_required bool
  stale_after_duration
```

### 19.3 Dedupe Strategy

Use layered matching:

- Provider external ID.
- Account.
- Posted date.
- Amount.
- Description fingerprint.
- Pending-to-posted transition logic.
- User-reviewed duplicate merge.

### 19.4 Data Quality Badges

Show:

- Fresh.
- Stale.
- Manual.
- Imported.
- Estimated.
- Needs review.
- Low confidence.
- Connector error.
- Reconciled.
- Unreconciled.
- Balance mismatch.
- Statement imported.
- Month closed.

### 19.5 Backup and Recovery

- Encrypted export package.
- Manual backup reminders.
- Optional backup to user-selected folder.
- Restore flow tested in CI with sample vaults.
- Password warning: no password reset for local vault.
- Consider optional recovery key generated at setup, encrypted separately, if design is reviewed.
- Add a Recovery Packet workflow before implementing emergency access.
- Add periodic restore drills: the app reminds the user to verify that an encrypted backup can be restored before they rely on the app for real data.

### 19.5.1 Data Portability and Export Formats

In addition to encrypted vault backup/restore, the app must support plaintext
data export in standard formats for interoperability and user freedom.

Required export formats:

- **Transactions CSV**: all transactions with date, amount, category, merchant,
  account, tags, notes. Compatible with spreadsheet import.
- **OFX/QFX**: for import into other personal finance tools.
- **JSON**: full structured export of all entities for programmatic use.
- **Tax summary CSV/PDF**: annual income, deductions, and category totals
  suitable for tax preparation review.
- **Net worth snapshot CSV**: point-in-time asset and liability summary.

Export rules:

- Plaintext exports require vault unlock and explicit user action.
- Exports include a generation timestamp and schema version.
- The app warns that plaintext exports are unencrypted.
- Exports are written to a user-selected folder, not a temp directory.
- Partial exports (single account, date range, category) are supported.
- Export should be automatable via a CLI or scheduled job for advanced users.

### 19.6 Reconciliation Strategy

The app should support lightweight personal-finance reconciliation without turning into enterprise accounting software.

Workflow:

1. User imports or enters statement period.
2. App compares opening balance, closing balance, transactions, fees, interest, payments, and pending items.
3. App proposes matches, duplicates, missing transactions, and adjustments.
4. User confirms or creates reconciliation adjustments.
5. Closed periods become locked by default, with explicit unlock/amend flow.

Acceptance criteria:

- Reconciled accounts show the statement date and balance.
- Forecasts warn when liquid-cash accounts have stale or unreconciled balances.
- Month-end close produces a concise household finance summary.

### 19.7 Local Job Runtime, Projection Cursors, and Performance Budgets

The app should include a small local job runtime. Jobs are not just background conveniences; they are the reliability layer for imports, projections, forecasts, backup, and diagnostics.

Job types:

- import parse
- import dedupe
- connector sync
- merchant normalization
- categorization
- recurring detection
- forecast generation
- forecast actualization
- backtest generation
- document extraction
- backup export
- read-model refresh
- vault health check
- notification scheduling
- notification delivery check

Rules:

- Jobs are durable inside the encrypted vault.
- Jobs are cancelable where possible.
- Jobs never run cloud AI or connector sync without explicit user settings.
- Job logs use safe redacted metadata only.
- Failed jobs create actionable Money Inbox items.
- Long-running jobs checkpoint progress and can resume safely after app restart.
- Job execution respects battery/network/privacy settings.
- The notification scheduler runs as a periodic local job that evaluates upcoming events and risk flags against user notification preferences and queues platform notifications.
- Notification scheduling does not require the app to be in the foreground; it runs on app launch and periodically while the app is open.
- If the app was not open when a notification was due, the notification is delivered on next app launch with appropriate staleness labeling.

Read models:

- `ledger_rows_read_model`
- `dashboard_summary_read_model`
- `cash_projection_read_model`
- `monthly_category_summary_read_model`
- `account_balance_read_model`
- `review_queue_read_model`
- `search_index_read_model`
- `change_journal_read_model`
- `money_inbox_read_model`
- `forecast_quality_read_model`

Read model rebuild rules:

- Every read model has a projection cursor tied to `operation_log.sequence_id`.
- Incremental updates are the default; full rebuild is a repair operation.
- Rebuild produces a checksum; divergence from the live model triggers a warning and repair flow.
- CI includes rebuild-and-compare tests for each read model.
- Rebuild should complete in under 30 seconds for a vault with 5 years of ordinary household data.
- User-facing queries should meet explicit budgets: dashboard under 250 ms warm, ledger page under 200 ms warm, Future Cash initial load under 500 ms warm for realistic fixture vaults.

---

## 20. Testing and Quality Strategy

### 20.1 Test Categories

| Test type | Purpose |
|---|---|
| Unit tests | Money math, schedules, categorization, rules |
| Property-based tests | Cashflow invariants, date edge cases, split sums |
| Migration tests | Upgrade/downgrade safety and fixture vaults |
| Encryption tests | Wrong password, key rotation, attachment encryption |
| Integration tests | Importers, connector mocks, database layer |
| UI tests | Onboarding, ledger, Future Cash, review queues |
| Performance tests | 100k+ transactions, large imports, chart rendering |
| Fuzz tests | CSV/OFX/QFX/QIF parsers, document metadata parsers |
| Forecast backtests | Historical forecasts vs actuals |
| Agent tests | Schema validation, prompt-injection resistance, cost caps |
| Security tests | IPC permissions, CSP, logging redaction |
| Fault-injection tests | power loss, interrupted migrations, interrupted key rotation, partial import commits |
| Golden-vault tests | fixture vaults with known forecasts, reconciliations, projections, and backtest outputs |
| Supply-chain tests | Dependency scanning, license checks, SBOM |

### 20.2 Forecast Invariants

Examples:

- Sum of splits equals transaction amount.
- Daily ending balance equals previous ending balance plus row effects.
- Transfers do not change aggregate household cash when both sides are included.
- Credit card purchases do not reduce cash until payment, unless debit/cash account.
- Investment balance changes do not affect liquid cash unless modeled as cashflow.
- Forecast assumptions are versioned.

### 20.3 Security Tooling

CI should include:

- `cargo test`.
- Rust formatting and clippy.
- TypeScript typecheck.
- Frontend linting.
- Dependency scanning.
- `cargo audit` or equivalent.
- npm/pnpm audit with policy.
- Secret scanning with gitleaks or trufflehog.
- Semgrep or CodeQL.
- License scanning.
- SBOM generation, CycloneDX or SPDX.
- Release artifact checksum generation.
- Tauri capability audit.
- CSP audit.
- Window/webview trust-boundary audit.
- Production config audit for devtools, remote assets, global Tauri exposure, and IPC allowlist drift.

### 20.4 External Review Gates

Before public release:

- Threat model reviewed.
- Vault/encryption design reviewed.
- IPC surface reviewed.
- Connector relay reviewed.
- Logs audited for sensitive data.
- Dependencies audited.
- Release signing tested.
- At least one external security review or penetration test completed.

---

## 21. Open Source and Release Strategy

### 21.1 Repository Structure

```text
[project-name]/
  apps/
    desktop/                 # Tauri app
      src/                   # React frontend
      src-tauri/             # Rust backend
  crates/
    core-money/
    core-ledger/
    forecast-engine/
    categorization/
    importers/
      importer-core/           # ImporterPlugin trait and registry
      importer-csv-generic/
      importer-ofx/
      importer-qif/
      importer-chase-csv/
      importer-schwab-csv/
      importer-coinbase-csv/
      importer-mint-csv/
      importer-ynab-csv/
    vault-crypto/
    agent-runtime/
    connector-core/
  services/
    connector-relay/
  docs/
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
    ISSUE_TEMPLATE/
  SECURITY.md
  CONTRIBUTING.md
  CODE_OF_CONDUCT.md
  LICENSE
  README.md
```

### 21.2 Required Docs

Create early:

```text
docs/product/vision.md
docs/product/personas.md
docs/product/mvp-scope.md
docs/product/non-goals.md
docs/architecture/system-overview.md
docs/architecture/trust-boundaries.md
docs/architecture/data-model.md
docs/architecture/connector-relay.md
docs/architecture/forecast-engine.md
docs/architecture/operation-log-and-projections.md
docs/architecture/performance-budgets.md
docs/architecture/connector-provider-registry.md
docs/architecture/headless-cli.md
docs/security/threat-model.md
docs/security/encryption-design.md
docs/security/ai-agent-safety.md
docs/security/logging-policy.md
docs/security/release-gates.md
docs/adr/0001-tauri-rust-react.md
docs/adr/0002-local-vault-design.md
docs/adr/0003-connector-relay-boundary.md
docs/adr/0004-future-cash-first-mvp.md
docs/adr/0005-agent-broker.md
docs/adr/0006-sync-conflict-resolution-strategy.md
docs/adr/0007-forecast-language-and-non-advice-boundary.md
```

### 21.3 License

Do not default to AGPLv3 until the project goal is clearer. License choice affects contributor willingness, commercial optionality, relay deployment, downstream packaging, and whether a future managed service is viable.

Recommended decision process:

| Goal | Better default | Rationale |
|---|---|---|
| Maximum user/developer adoption | Apache 2.0 or MIT | Low friction, easy packaging and corporate contribution |
| File-level copyleft without scaring off most contributors | MPL 2.0 | Protects core files while allowing broader ecosystem use |
| Strong copyleft for distributed desktop forks | GPLv3 | Ensures app fork source availability |
| Prevent closed hosted relay forks | AGPLv3 for relay/service crates | Targets network-service modifications where it matters |
| Future commercial sustainability | Dual license or open-core split | Requires early governance clarity |

Proposed default unless a strong copyleft goal is non-negotiable:

- Desktop app and core crates: **MPL 2.0 or Apache 2.0**.
- Optional hosted/self-hosted relay: **AGPLv3 or separate commercial-compatible license**, depending on business intent.
- Decide before public repository launch and document the rationale in an ADR.

Final license decision should happen before public repository launch.

### 21.4 Public Release Gates

Do not open the repo until:

- No real financial data exists in git history.
- No secrets exist in git history.
- Threat model is complete.
- Encryption design is documented.
- `SECURITY.md` is complete.
- Vulnerability disclosure process exists.
- Dependency/license audit is complete.
- CI security checks pass.
- Logs are verified to exclude sensitive data.
- App is signed, hardened, and notarized for macOS distribution.
- Update signing works.

Private beta release gates:

- Developer ID signing works.
- Hardened runtime enabled with minimum exceptions.
- App Sandbox entitlements documented.
- Notarization succeeds for private beta artifacts.
- Auto-update is disabled unless update signing and rollback behavior are tested.
- Release artifact checksums are generated and stored.
- Signing/notarization credentials are not stored in the repository or ordinary developer shell history.
- Connector relay does not contain known high/critical vulnerabilities.
- AI agents are off by default or explicitly consented.
- Sample dataset generator exists.
- Demo vault uses synthetic data only.
- External security review findings are addressed or documented.

### 21.5 Branding

Working codename: **[TBD]**.

Before public launch:

- Trademark search.
- Verify no active registrations in software/finance classes.
- GitHub org/repo availability.
- Domain availability.
- Package namespace availability.
- crates.io and npm namespace availability.
- App icon and privacy-forward branding.

---

## 22. Roadmap

The roadmap should be controlled by release gates, not elapsed weeks. Week ranges are planning hints only. A later gate may not begin until the previous gate passes its acceptance criteria with a real fixture vault and a dogfood vault.

Phase gate rules:

- Do not start connector work until manual/import forecasting is useful.
- Do not start cloud AI until deterministic insights, evidence records, and report storage work locally.
- Do not start investment depth until Future Cash, ledger, and reconciliation are stable.
- Do not open source until security docs, fixture vaults, and release-signing workflows are complete.
- Any phase can ship privately only after backup/restore and redaction tests pass.

### Phase 0: Foundation and Architecture, Weeks 1-6

Deliverables:

- Tauri v2 app scaffold.
- React/TypeScript frontend shell.
- Rust command layer.
- SQLCipher integration spike.
- Vault creation/unlock prototype.
- Finance Kernel skeleton.
- Ledger transaction/posting model draft.
- Semantic money type.
- Source/provenance model draft.
- Ingestion staging model draft.
- Materialized read-model pattern.
- Data model draft.
- Threat model draft.
- Synthetic household data generator.
- CI baseline.
- Product and architecture docs.
- Command idempotency model.
- Operation log/projection cursor prototype.
- Vault state-machine sketch and startup self-test prototype.
- Golden synthetic fixture vault v0.

Milestone (Week 4):

> Foundation crates compile, vault creates and unlocks, basic ledger postings work.

### Phase 0.5: Week 8 First Playable, Weeks 7-8

Deliverables:

- Manual accounts and balances.
- Manual transaction entry.
- Manual salary income schedule.
- Manual recurring bills (flat list).
- Deterministic Future Cash ledger.
- Minimal dashboard: cash, bills, income, 30-day forecast.
- Encrypted backup.
- Begin personal dogfooding with real financial data.

Milestone (Week 8 — HARD DEADLINE):

> The developer is using the app daily for personal cash forecasting. The app is ugly but correct and useful. Kill/continue decision based on personal utility.

### Gate R1: Local Manual Cash Forecast (Phase 1: Local Manual Finance Tracker, Weeks 9-14)

Goal:

> The app is personally useful for daily cash planning without imports, connectors, documents, AI, or investments.

Must pass:

- encrypted vault create/unlock/lock
- manual liquid accounts
- opening balances as ledger/equity postings
- manual salary income
- manual recurring bills
- deterministic 30/90-day Future Cash
- dashboard with cash, next income, next bills, forecast mini-chart
- encrypted backup and verified restore
- redaction tests pass
- no real-data dogfooding before backup/restore and log-redaction tests pass

Deliverables:

- CSV import with column mapping.
- CSV staging/preview/commit pipeline.
- Initial OFX/QFX/QIF parser spike.
- Full category taxonomy.
- Tags, notes, splits.
- Touch ID unlock.
- Quick-entry templates.
- Encrypted attachments foundation.
- Backup/export v1.
- Reconciliation session v0.

Milestone:

> Functional local encrypted personal finance tracker with manual/imported data, staged import review, auditable ledger commits, and basic reconciliation.

### Gate R2: Importable Local Finance Tracker (Phase 2: Income, Recurring Bills, and Deterministic Future Cash, Weeks 11-16)

Goal:

> A user can import files, review staged candidates, commit to the ledger, reconcile balances, and regenerate the same forecast from canonical data.

Must pass:

- CSV import with column mapping
- source records and provenance links
- staged candidate review
- idempotent import commit plan
- balance observations
- reconciliation v0
- transaction display read model
- forecast explainability
- golden-vault rebuild test

Deliverables:

- Income source setup.
- Salary/hourly/contractor income models v1.
- Pay schedule engine.
- Recurring bill setup.
- Bill contract model.
- Recurring detection v1.
- Deterministic Future Cash ledger.
- Forecast input snapshots.
- Forecast reproducibility metadata.
- Manual future entries.
- Basic dashboard.
- Forecast row explanations.
- Notification infrastructure remains a spike only. User-facing notifications do not ship until the app has Forecast Readiness, stale-data warnings, and at least one verified backup/restore path.

Milestone:

> User can see expected future cash from known income, known bills, and manual assumptions.

### Gate R3: Learning Forecast (Phase 3: Intelligent Forecasting, Categorization, and Risk Flags, Weeks 17-26)

Goal:

> The app learns from historical transactions and produces evidence-backed variable-spend estimates, credit-card payment forecasts, and risk flags.

Merge of original Phases 3 and 4. Categorization intelligence (merchant normalization,
rules engine, transfer detection, local classifier) is a prerequisite for good variable
spending forecasts. Building them in the same phase creates a tighter feedback loop.

Deliverables:

- Variable spending forecasts.
- Credit card cycle/payment forecast.
- Confidence bands.
- Change-point/trend detection spike.
- Risk flag engine.
- Forecast backtesting framework.
- Forecast calibration metrics.
- Simulation engine v1 for variable/lumpy spending.
- Future Cash ledger + chart UI.
- Scenario overlays.
- Merchant normalization.
- Transfer detection.
- Advanced user rules.
- Local classifier v1.
- Split templates.
- Review queues.
- Bulk cleanup tools.
- Categorization evaluation metrics.

Milestone:

> The crown jewel works: a forward-looking cashflow statement with risk flags and confidence bands, fed by categorization that learns from the user.

### Phase 4: Connector Adapters and Documents, Weeks 27-36

Run connector and document work in parallel. They are independent workstreams
that both feed into the ingestion pipeline.

Deliverables:

- SimpleFIN adapter or equivalent.
- Teller adapter spike if practical.
- Plaid BYOK/self-hosted relay prototype.
- Connector provider interface.
- Signed connector batch manifest.
- Connector batches routed through ingestion staging.
- Connection health dashboard.
- Stale-data handling.
- Dedupe/reconciliation.
- Reauth messaging.
- Pay stub upload and manual extraction workflow.
- Statement/document library.

Milestone:

> Automated sync works where configured, the document library accepts statements and pay stubs, and the app remains robust when connections fail.

### Phase 5: Investments, Debt, and Agent System, Weeks 37-46

Merge of original Phases 6 and 7. The agent system depends on forecast + categorization
(both done by now). Investment/debt views and agents can be built concurrently.

Deliverables:

- Credit/debt detail views.
- Debt payoff scenarios.
- Investment holdings and snapshots.
- Net worth dashboard.
- Alternative/manual assets.
- Agent broker.
- Agent definitions.
- BYOK key management.
- Local model abstraction.
- Risk Agent.
- Cash Runway Agent.
- Planning Agent.
- Bill Agent.
- Agent report library.
- Evidence citations.
- Cost controls.
- Prompt-injection tests.

Milestone:

> The app becomes a full household financial statement tool with scoped, evidence-backed AI agents.

### Phase 6: Polish, Hardening, and Beta, Weeks 47-54

Deliverables:

- Configurable dashboard.
- Onboarding polish.
- Accessibility pass.
- Performance optimization.
- Error states.
- Migration hardening.
- Backup/restore testing.
- Security audit preparation.
- Signed and notarized private beta release pipeline.
- Update-signature verification test, even if auto-update remains disabled.
- User documentation.

Milestone:

> Private beta-quality app suitable for dogfooding and limited trusted testers.

### Phase 7: Open Source Release Prep, Weeks 55-60

Deliverables:

- External security review.
- Fix high/critical findings.
- Complete `SECURITY.md`.
- Complete architecture docs.
- License finalization.
- Public README.
- Contribution guide.
- Code of conduct.
- Issue templates.
- Signed/notarized release artifacts.
- Public roadmap.

Milestone:

> Public open-source release with honest security posture and stable core functionality.

Total timeline: ~60 weeks (down from 72 by parallelizing categorization with forecasting,
connectors with documents, and investments/debt with agents).

---

## 23. Initial GitHub Issues

Create these in the private repo.

### Architecture and Security

1. `ADR: Select Tauri v2 + Rust + React architecture`
2. `ADR: Define local encrypted vault model`
3. `ADR: Define frontend/backend trust boundary`
4. `ADR: Define connector relay boundary`
5. `ADR: Make Future Cash the first product wedge`
6. `ADR: Define Finance Kernel and command boundary`
7. `ADR: Define ledger transaction/posting model`
8. `ADR: Define staged ingestion and provenance model`
9. `ADR: Define materialized read-model strategy`
10. `ADR: Define Tauri window/capability isolation model`
11. `Create threat model v1`
12. `Create encryption design doc`
13. `Create logging and local observability redaction policy`
14. `Create secure release gate checklist`
15. `Add gitleaks/trufflehog secret scanning`
16. `Add dependency scanning for Rust and frontend`
17. `Add SBOM generation`

Additional architecture/security issues to add before implementation:

- `ADR: Choose hybrid ledger + operation log persistence model`
- `ADR: Define command idempotency and retry semantics`
- `ADR: Define parser/document isolation boundary`
- `ADR: Define Money Inbox and deterministic insight primitives`
- `ADR: Define connector capability/cost/terms registry`
- `ADR: Define headless CLI safety model`
- `ADR: Define sync-readiness fields (node_id, HLC) and conflict resolution principles`

### Core App

18. `Scaffold Tauri desktop app`
19. `Add React/TypeScript frontend shell`
20. `Implement Rust IPC command pattern with typed schemas`
21. `Implement Finance Kernel command bus`
22. `Implement semantic money type`
23. `Implement ledger transaction/posting tables`
24. `Implement ledger invariant tests`
25. `Integrate SQLCipher prototype`
26. `Implement vault creation`
27. `Implement vault unlock/lock`
28. `Implement Argon2id key derivation`
29. `Implement Keychain storage for wrapped biometric unlock secret`
30. `Implement auto-lock timer`
31. `Add database migration framework`
32. `Add materialized read-model refresh pattern`
33. `Add local durable job runtime`
33.5 `Implement local notification system with Tauri notification plugin`
33.6 `Build notification preference UI and scheduling engine`
33.7 `Add bill-due, paycheck, and risk-flag notification triggers`

### Data and Manual Mode

34. `Create account schema and CRUD through Finance Kernel`
35. `Create ledger-backed transaction projection`
36. `Create category taxonomy`
37. `Create tags and notes support`
38. `Create transaction split support over postings`
39. `Build source batch and source record tables`
40. `Build CSV import column mapper`
41. `Build CSV staging preview`
42. `Build import commit plan`
43. `Add OFX/QFX/QIF parser spike`
44. `Create balance snapshot model`
45. `Create reconciliation session v0`
46. `Create encrypted backup/export v1`

### Forecasting

47. `Create income source schema`
48. `Implement pay schedule engine`
49. `Implement recurring event schema`
50. `Build manual recurring bill UI`
51. `Implement deterministic Future Cash ledger`
52. `Add forecast row explanations`
53. `Add manual future entries`
54. `Implement credit card cycle model`
55. `Implement card statement forecast v1`
56. `Create variable spending forecast prototype`
57. `Create risk flag schema`
58. `Build forecast backtesting framework`
59. `Implement forecast input snapshots and model registry`
60. `Implement Monte Carlo simulation engine v1`
60.5 `Build Financial Calendar view over forecast read model`

### Categorization

61. `Implement merchant normalization v1`
62. `Implement user rule engine`
63. `Implement transfer detection v1`
64. `Build low-confidence review queue`
65. `Implement local categorization classifier prototype`
66. `Add split templates for ambiguous merchants`

### Connectors

67. `Define provider-neutral connector interface`
68. `Build connector mock provider for tests`
69. `Research SimpleFIN adapter feasibility`
70. `Implement SimpleFIN adapter spike`
71. `Research Teller adapter feasibility`
72. `Design self-hosted connector relay`
73. `Define signed connector batch manifest`
74. `Implement Plaid relay sandbox spike`
75. `Build connection health dashboard`
76. `Implement stale-data warnings`
77. `Implement transaction dedupe/reconciliation through ingestion pipeline`

### UX and Product

78. `Design onboarding flow`
79. `Build dashboard widget framework`
80. `Build Future Cash ledger/chart wireframe`
81. `Build accounts overview`
82. `Build transaction ledger with virtualization on read models`
83. `Build settings/privacy mode`
84. `Add keyboard shortcuts`
85. `Accessibility audit v1`
86. `Build Reconciliation Center v1`
87. `Build "What changed since last open" daily check-in`

### Agents and Documents

88. `Design agent broker with deterministic analyzer + narrator split`
89. `Define agent output schema (findings/evidence/assumptions/actions)`
90. `Implement BYOK API key storage with Keychain`
91. `Implement Risk Agent deterministic analyzer on synthetic data`
92. `Implement agent evidence citations`
93. `Create agent report library`
94. `Implement document storage schema`
95. `Build pay stub manual extraction workflow`
96. `Add prompt-injection test fixtures`
97. `Implement isolated document_preview WebView surface`

---

## 24. Risk Register

| Risk | Severity | Likelihood | Exposure | Mitigation | Verification |
|---|---:|---:|---|---|---|
| Scope creep | High | High | Months of delay, no shippable product | Future Cash MVP first; explicit non-goals | Phase gate reviews at each milestone |
| WebView security bug | High | Medium | Vault data exposure via frontend | Rust trust boundary, CSP, Tauri capabilities, no secrets in frontend | IPC audit coverage metric; CSP test in CI |
| User loses master password | High | Medium | Permanent data loss | Clear warning, encrypted backups, optional recovery design after review | Onboarding flow includes backup prompt; recovery key UX tested |
| Bank connector instability | High | High | Degraded user experience, stale forecasts | Manual mode, imports, stale-data handling, multiple providers | Forecast continues with stale-data label; manual override always available |
| Provider pricing/API changes | High | Medium | Connector becomes unusable or unaffordable | Provider-neutral interface, BYOK, relay abstraction | Provider registry includes pricing review dates |
| Forecast inaccuracy | High | Medium | User makes bad financial decisions | Backtesting, confidence bands, explainability, conservative defaults | Backtest error metrics tracked per release |
| SQLCipher performance | Medium | Medium | Slow UI on large vaults | Indexing, pagination, virtual tables, performance tests | Performance budget tests in CI with fixture vaults |
| Large ledger UI performance | Medium | Medium | Unusable transaction view | Virtualized data grid, query pagination | 100k-transaction fixture renders under 200ms |
| Local ML poor accuracy | Medium | Medium | Bad categorization, user frustration | Rules first, review queue, user feedback loop | Categorization precision/recall metrics tracked |
| LLM cost overruns | Medium | Medium | Unexpected API charges | Hard caps, cost preview, quotas, no background runs by default | Cost cap enforcement tested with mock provider |
| Prompt injection | High | Medium | Agent produces misleading financial report | Structured inputs, no tools, schema validation, read-only agents | Prompt injection test fixtures in CI |
| Supply-chain compromise | High | Medium | Malicious code in dependency | Lockfiles, SCA, SBOM, secret scanning, review discipline | CI blocks on critical vulnerability findings |
| Public release before ready | High | Medium | Security incident with real user data | Release gates and external review | Gate checklist is a merge-blocking CI check |
| Legal/regulatory ambiguity | Medium | Medium | Cease-and-desist or compliance issue | Avoid payment initiation/advice; monitor open banking rules; disclaimers | Legal watchlist reviewed quarterly |
| Tax/advice liability | Medium | Medium | User claims reliance on app output | Estimates only, user assumptions, no professional advice claims | Disclaimer text reviewed by counsel before public release |
| Transaction-centric schema becomes limiting | High | Medium | Costly data model migration | Use ledger postings and semantic money type early | Schema review at Phase 2 gate |
| Import pipeline commits bad data irreversibly | High | Medium | Corrupted financial history | Stage imports, preview commit plans, store provenance, support amendments | Import rollback tested in CI |
| SQLite side files or temp files leak sensitive data | High | Low/Medium | Plaintext financial data on disk | Explicit SQLCipher/WAL/temp-file tests and no plaintext document temp files | WAL/SHM/temp-store redaction test in CI |
| Untrusted document or LLM output compromises WebView | High | Medium | XSS or data exfiltration | Separate no-IPC preview surfaces, CSP, sanitization, capability isolation | CSP and sanitization tests for each WebView surface |
| Forecast percentiles are misunderstood | Medium | Medium | User overreacts or ignores warnings | Label cash outcomes clearly, show assumptions, explain downside drivers | User-tested copy for P10/P50/P90 labels |
| Materialized read models drift from canonical ledger | Medium | Medium | Dashboard shows wrong data | Deterministic rebuilds, checksums, migration tests, invariant checks | Rebuild-and-compare test in CI |
| Recovery key weakens vault security | High | Low/Medium | Attacker gains vault access via recovery key | Optional design, high entropy, one-time display, external review, strong UX warnings | External cryptographic review before feature ships |
| Primary user incapacitation/death locks out household | High | Low/Medium | Partner loses access to financial data | Emergency access design, recovery key education, encrypted backup with shared storage guidance | Household documentation checklist in app |
| Solo developer motivation decay | High | High | Project abandoned before useful | Ship usable MVP by week 16, dogfood immediately, seek early testers by week 30 | Weekly progress log; phase gate reviews |
| Argon2id parameter regression | High | Low | Vault encryption silently weakened | KDF parameters validated at vault creation and rekey; CI test asserts minimum parameters | CI test: `assert memory >= 64 MiB && time_cost >= calibrated_minimum` |
| SQLCipher version incompatibility | High | Low/Medium | Old vaults unreadable after upgrade | Pin SQLCipher version; test vault open across versions; migration path for cipher changes | Golden vault fixtures include vaults created with previous SQLCipher versions |
| Tauri v2 breaking changes | Medium | Medium | App build or capability model breaks | Pin Tauri version; test upgrades in CI branch before merging | Tauri version upgrade is a dedicated PR with full test pass |
| Solo developer bus factor | High | Medium | Project has no succession plan | Document architecture decisions thoroughly; keep README and CONTRIBUTING current; identify 1-2 potential co-maintainers by Phase 5 | Architecture docs sufficient for a new developer to build and modify the app |

---

## 25. Metrics and Acceptance Criteria

### 25.1 Product Metrics

- Time to first useful dashboard.
- Time to first Future Cash forecast.
- Percent of transactions auto-categorized above confidence threshold.
- User correction rate by category.
- Forecast error at 7/30/90 days.
- Number of stale connector days.
- Reauth recovery success rate.
- Risk flag usefulness rating, later beta.
- Import success rate.

### 25.2 Security Metrics

- Critical/high vulnerabilities open.
- Secret scanning pass/fail.
- Dependency scan pass/fail.
- SBOM generated per release.
- Redaction test pass/fail.
- Vault wrong-password test pass/fail.
- Attachment encryption test pass/fail.
- IPC command audit coverage.
- External audit findings closed.

### 25.3 MVP Acceptance Criteria

MVP is successful when:

- A user can manually enter/import data and get a useful forecast.
- The vault is encrypted and password-protected.
- The app is useful offline.
- Forecast assumptions are editable.
- The dashboard shows current cash, upcoming bills, upcoming income, and cash forecast.
- Transaction categorization is at least rule-based and correctable.
- Backup/export works.
- There is no plaintext financial data in logs.

---

## 26. What to Defer

Do not build these early:

- Mobile app.
- Cloud sync.
- Hosted premium service.
- Household multi-user real-time collaboration (read-only snapshots are the interim solution).
- Payment initiation.
- Brokerage trading.
- Investment recommendations.
- Tax filing.
- Bill pay.
- Full tax optimization.
- Marketplace/plugins.
- Open-ended chatbot.
- Global model training on user data.
- Complex role-based permissions.
- Real-time market data.
- Multi-currency tax lots.

Also do not build:

- Autonomous financial actions.
- Auto-submitted payments, transfers, trades, loan applications, or account changes.
- Personalized investment recommendations framed as advice.
- Tax advice or tax filing decisions.
- Cloud identity as a prerequisite for local vault use.
- Hidden telemetry or product analytics.
- Background cloud AI analysis without explicit consent.
- Marketplace/plugin execution against real vault data.

The product may provide calculations, forecasts, explanations, reminders, and user-confirmed planning suggestions. It should not present itself as a regulated financial, investment, legal, or tax advisor.

These features either increase regulatory/security burden or distract from the core product wedge.

---

## 27. Immediate Next Steps

1. Create private repo using the proposed monorepo structure.
2. Add `docs/adr/0001-tauri-rust-react.md` explaining the stack decision.
3. Add `docs/security/threat-model.md`.
4. Scaffold Tauri app.
5. Implement a minimal encrypted SQLCipher vault.
6. Implement account and transaction CRUD against encrypted DB.
7. Build synthetic household data generator.
8. Build deterministic Future Cash prototype with manual income and bills.
9. Start dogfooding with synthetic data first. Personal-data dogfooding is blocked until the real-data safety gate passes.
10. Defer automated bank connections until the manual product is genuinely useful.

---

## 28. Footnotes

[^tauri-capabilities]: Tauri v2 capabilities documentation: https://v2.tauri.app/security/capabilities/
[^apple-keychain]: Apple Keychain Services documentation: https://developer.apple.com/documentation/security/keychain-services
[^apple-localauth]: Apple LocalAuthentication documentation: https://developer.apple.com/documentation/localauthentication/
[^apple-app-sandbox]: Apple App Sandbox documentation: https://developer.apple.com/documentation/security/app-sandbox
[^apple-hardened-runtime]: Apple Hardened Runtime documentation: https://developer.apple.com/documentation/security/hardened-runtime
[^apple-notarization]: Apple notarization documentation: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
[^sqlcipher]: SQLCipher official site: https://www.zetetic.net/sqlcipher/
[^plaid-link]: Plaid Link overview: https://plaid.com/docs/link/
[^plaid-hosted-link]: Plaid Hosted Link documentation: https://plaid.com/docs/link/hosted-link/
[^plaid-webview]: Plaid webview integration documentation: https://plaid.com/docs/link/webview/
[^simplefin]: SimpleFIN protocol documentation: https://www.simplefin.org/protocol.html
[^teller]: Teller API documentation: https://teller.io/docs
[^fdx-cfpb]: CFPB recognition of Financial Data Exchange as a standard-setting body: https://www.consumerfinance.gov/about-us/newsroom/cfpb-approves-application-from-financial-data-exchange-to-issue-standards-for-open-banking/
[^ecfr-1033]: eCFR 12 CFR Part 1033 Personal Financial Data Rights: https://www.ecfr.gov/current/title-12/chapter-X/part-1033
[^cfpb-reconsideration]: CFPB Personal Financial Data Rights reconsideration page: https://www.consumerfinance.gov/rules-policy/rules-under-development/personal-financial-data-rights-reconsideration/
[^nist-ssdf]: NIST SP 800-218 Secure Software Development Framework: https://csrc.nist.gov/pubs/sp/800/218/final
[^owasp-asvs]: OWASP Application Security Verification Standard: https://owasp.org/www-project-application-security-verification-standard/
[^owasp-masvs]: OWASP MASVS: https://mas.owasp.org/MASVS/
[^owasp-scvs]: OWASP Software Component Verification Standard: https://owasp.org/www-project-software-component-verification-standard/
