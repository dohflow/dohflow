# Architecture Decision Records — index

One number sequence spans **both** repositories: this one
(`dohflow/dohflow`, public) and `dohflow/internal` (private). An **internal**
entry below has no title and no file here — the decision, its reasoning, and
its file live only in `dohflow/internal`, and this repo's tree never contains
business detail for it. When an internal decision ships as a user-visible
feature, a **public stub ADR** appears here at that same number, stating the
accepted decision in the site's own words and nothing more (see ADR 0082,
decision 3).

Tier is decided **before** an ADR is written (ADR 0082, decision 4) and
recorded here — this index, not a per-file header, is the source of truth for
the sixty-three ADRs that predate ADR 0082 (all of them public; see ADR
0082's own decision 4 for why they aren't retroactively stamped). Every ADR
from 0082 onward states its own tier in its front matter, and that value must
match this table.

Numbers 0005, 0016, 0017, and 0019 were never assigned — the sequence simply
skips them; that's a pre-existing gap, not an error. **0015 is not part of
that gap** — `personal-cfo-j5d` held that slot since 2026-05-03, predating
this note; the note was wrong about 0015 specifically, corrected 2026-09-19
when the ADR was written (owner-confirmed).

| # | Title | Tier | Status |
|---|---|---|---|
| 0001 | [Tauri v2 + Rust core + React/TypeScript frontend](0001-tauri-rust-react.md) | Public | Accepted |
| 0002 | [Local encrypted vault model](0002-local-encrypted-vault.md) | Public | Accepted |
| 0003 | [Frontend / backend trust boundary](0003-trust-boundary.md) | Public | Accepted (amended twice) |
| 0004 | [Connector relay boundary](0004-connector-relay-boundary.md) | Public | Accepted |
| 0006 | [Finance Kernel and command boundary](0006-finance-kernel-command-boundary.md) | Public | Accepted |
| 0007 | [Ledger transaction / posting model](0007-ledger-transaction-posting-model.md) | Public | Accepted |
| 0008 | [Staged ingestion and provenance model](0008-staged-ingestion-and-provenance.md) | Public | Accepted |
| 0009 | [Materialized read-model strategy](0009-materialized-read-model-strategy.md) | Public | Accepted |
| 0010 | [Tauri window + capability isolation model](0010-tauri-window-capability-isolation.md) | Public | Accepted (amended) |
| 0011 | [Hybrid ledger + operation-log persistence](0011-hybrid-ledger-operation-log.md) | Public | Accepted |
| 0012 | [Command idempotency and retry semantics](0012-command-idempotency-and-retry.md) | Public | Accepted |
| 0013 | [Entity identifier strategy](0013-id-strategy.md) | Public | Accepted |
| 0014 | [Money Inbox and the ingestion pipeline](0014-money-inbox-and-ingestion-pipeline.md) | Public | Accepted |
| 0015 | [Connector capability/cost/terms registry](0015-connector-capability-cost-terms-registry.md) | Public | Accepted |
| 0018 | [Forecast language and non-advice boundary](0018-forecast-language-and-non-advice-boundary.md) | Public | Accepted |
| 0020 | [Frontend state, data-fetching, and forms architecture](0020-frontend-state-data-forms.md) | Public | Accepted |
| 0021 | [Date and timezone policy](0021-date-timezone-policy.md) | Public | Accepted |
| 0022 | [Parser / document isolation boundary](0022-parser-document-isolation.md) | Public | Accepted |
| 0023 | [Encrypted attachment store](0023-encrypted-attachment-store.md) | Public | Accepted |
| 0024 | [Encrypted backup/restore format](0024-encrypted-backup-restore-format.md) | Public | Accepted |
| 0025 | [Multi-vault management (registry, naming, lifecycle)](0025-multi-vault-management.md) | Public | Accepted |
| 0026 | [Forecast architecture](0026-forecast-architecture.md) | Public | Accepted |
| 0027 | [Additive balance model](0027-additive-balance-model.md) | Public | Accepted |
| 0028 | [Account subtypes and cash-tier rollups](0028-account-subtype-and-cash-tiers.md) | Public | Accepted |
| 0029 | [Cash availability model](0029-cash-availability-model.md) | Public | Accepted |
| 0030 | [Transaction categorization](0030-categorization.md) | Public | Accepted |
| 0031 | [UI-quality discipline and the Claude Design workflow](0031-ui-quality-and-design-workflow.md) | Public | Accepted |
| 0032 | [Reviewed-state and the unified review queue](0032-reviewed-state-and-unified-review-queue.md) | Public | Accepted |
| 0033 | [Transaction tags and notes](0033-transaction-tags-and-notes.md) | Public | Accepted |
| 0034 | [Transaction splits](0034-transaction-splits.md) | Public | Accepted |
| 0035 | [Debt-payment forecast model](0035-debt-payment-forecast-model.md) | Public | Accepted |
| 0036 | [Scenario / projection generalization](0036-scenario-projection-generalization.md) | Public | Proposed |
| 0037 | [Cross-account analytics information architecture](0037-cross-account-analytics-information-architecture.md) | Public | Proposed |
| 0038 | [Spend classification — ordinary vs extraordinary](0038-spend-classification-ordinary-extraordinary.md) | Public | Accepted |
| 0039 | [Credit-card cycle and statement-balance forecast](0039-credit-card-cycle-and-statement-forecast.md) | Public | Accepted |
| 0040 | [MLP-first roadmap](0040-mvp-release-criteria.md) | Public | Accepted |
| 0041 | [Bill autopay is intent/label, not a projection change](0041-bill-autopay-semantics.md) | Public | Accepted |
| 0042 | [Multi-vault management: subfolders + plaintext registry](0042-multi-vault-management.md) | Public | Accepted |
| 0043 | [OSS license: AGPL-3.0-only + CLA](0043-oss-license.md) | Public | Accepted |
| 0044 | [Account-model extensions](0044-account-model-extensions.md) | Public | Accepted |
| 0045 | [Ingestion field capture and dual-date semantics](0045-ingestion-field-capture-and-dual-date.md) | Public | Accepted |
| 0046 | [Recurring-suggestion dismissal and suppression](0046-recurring-suggestion-dismissal.md) | Public | Accepted |
| 0047 | [Recurring detection→promotion semantics](0047-recurring-detection-to-promotion-semantics.md) | Public | Accepted |
| 0048 | [Interval frequencies](0048-interval-frequencies.md) | Public | Accepted |
| 0049 | [Sidebar information architecture](0049-sidebar-information-architecture.md) | Public | Accepted |
| 0050 | [Forecast uncertainty model](0050-forecast-uncertainty-model.md) | Public | Accepted |
| 0051 | [Scenario lifecycle](0051-scenario-lifecycle.md) | Public | Accepted |
| 0052 | [Spend-by-category analytics on Transactions](0052-spend-analytics-placement.md) | Public | Accepted |
| 0053 | [List-surface convergence](0053-list-surface-convergence.md) | Public | Accepted |
| 0054 | [Categorical chart palette](0054-categorical-chart-palette.md) | Public | Accepted |
| 0055 | [Applying a scenario promotes, never rewrites](0055-applying-a-scenario.md) | Public | Accepted |
| 0056 | [An archived liquid account leaves the forecast](0056-archived-accounts-leave-the-forecast.md) | Public | Accepted |
| 0057 | [The Debt page's view contract](0057-debt-page-view-contract.md) | Public | Accepted |
| 0058 | [Surfacing a past-due unconfirmed obligation](0058-past-due-obligations-need-confirmation.md) | Public | Accepted |
| 0059 | [Composing several scenarios](0059-composing-several-scenarios.md) | Public | Accepted |
| 0060 | [Connector strategy: SimpleFIN first](0060-connector-strategy-simplefin-first.md) | Public | Accepted |
| 0061 | [Website stack and hosting](0061-website-stack-and-hosting.md) | Public | Accepted |
| 0062 | [Public-repo fork mechanics](0062-public-repo-fork-mechanics.md) | Public | Accepted (partially superseded by its own 2026-09-08 amendment) |
| 0063 | [Typeface pairing](0063-typeface-pairing.md) | Public | Accepted |
| 0064 | [Task tracking leaves the product repository](0064-task-tracking-leaves-the-repo.md) | Public | Accepted |
| 0065 | [Minimum supported macOS version](0065-minimum-macos-version.md) | Public | Accepted |
| 0066 | [Business model: free local app + paid services](0066-business-model-free-app-paid-services.md) | Public | Accepted — discusses the business model in the abstract; allowlisted in the tier tripwire (ADR 0082) |
| 0067 | [DohFlow rename policy](0067-dohflow-rename-policy.md) | Public | Accepted |
| 0068 | [Release distribution and update channel](0068-release-distribution-and-update-channel.md) | Public | Accepted |
| 0069 | [Bead-graph reconciliation rules](0069-bead-graph-reconciliation-rules.md) | Public | Accepted |
| 0070 | [Dev vs. release vault data-directory separation](0070-dev-release-vault-directory-separation.md) | Public | Accepted |
| 0071 | [Public-repo contribution model](0071-public-repo-contribution-model.md) | Public | Accepted |
| 0072 | [Universal binary distribution (Intel + Apple Silicon)](0072-universal-binary-distribution.md) | Public | Accepted |
| 0073 | Sync disposition rules | Public | Not yet written (`personal-cfo-vlfd`) |
| 0074 | [DohFlow Sync architecture](0074-dohflow-sync-architecture.md) | Public | Accepted |
| 0075 | — internal | Internal | Not yet written (`personal-cfo-oh3hg`) |
| 0076 | Multi-provider connector strategy (public stance) | Public + Internal split | Not yet written (`personal-cfo-m0kgx`) — public ADR carries the decision and the D15/FTC sentence; an internal companion carries the connector's affiliate-payout terms and demand-test results |
| 0077 | Data ownership and wire formats | Public | Not yet written (`personal-cfo-klr.4`) |
| 0078 | — internal | Internal | Not yet written (`personal-cfo-quhmv`) |
| 0079 | Desktop-first platform order | Public | Not yet written (`personal-cfo-bonpc`) |
| 0080 | Mobile architecture | Public | Not yet written (`personal-cfo-iuxhq`) |
| 0081 | — internal | Internal | Not yet written (`personal-cfo-658vi`) |
| 0082 | [The public-disclosure boundary](0082-public-disclosure-boundary.md) | Public | Accepted |

## Sub-numbered amendments

Referenced elsewhere as e.g. "ADR 0013-A" — an addendum to an existing ADR
rather than a new top-level number. All public tier: 0013-A, 0024-A, 0066-A.
