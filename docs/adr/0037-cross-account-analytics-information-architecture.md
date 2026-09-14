# ADR 0037: Information architecture for cross-account analytics — Accounts sub-views

- **Status:** Proposed
- **Date:** 2026-06-30
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-915.2`](../../.beads/issues.jsonl) (this ADR); gates `6wk.5`
  (debt-burndown + net-worth viz), `4d8.4` (account grouping), and the per-account
  debt/investment views
- **Builds on:** ADR 0020 (frontend state/data/forms — routing deferral), ADR 0028 (account
  subtype + cashflow role — the grouping key), ADR 0031 (UI quality), ADR 0018 (non-advice)

## Context

The debt + investments arc adds cross-account analytics — a debt-payoff **burndown**, a
**net-worth timeline**, per-account debt/investment detail views — and there is no agreed home
for them. The planning docs and beads carry **five conflicting answers**: top-level sections
(plan §18.1 / `sbm3`: *Accounts | Credit & Debt | Investments*), optional dashboard widgets
(§18.2), dashboard profiles (§18.3 / `mh93`), and the project owner's own proposal — **one
Accounts page with Liquid / Investments / Debt sub-views**. Building any chart before this is
settled implicitly picks the IA and risks rework (AGENTS.md §1A). The owner stated the
preference directly while dogfooding; this ADR ratifies it and reconciles the rest.

## Decision

### 1. One **Accounts** surface with role-based sub-views: Liquid · Investments · Debt

Accounts is a single top-level surface with **sub-views** partitioned by `cashflow_role`
(ADR 0028): **Liquid** (`liquid_cash`), **Investments** (`investment_asset`, later
`real_asset`), **Debt** (`credit_facility`, `loan_liability`). This is the canonical home for
per-account detail and the role's analytics — **not** three separate top-level sections (§18.1
/ `sbm3`) and **not** a primary dashboard home (§18.2/§18.3). It keeps "my accounts" in one
place the user navigates by role, matching the mental model and the owner's preference.

### 2. Where each analytic lives

- **Debt sub-view** hosts the **debt-payoff burndown** (projected balance-over-time per debt +
  aggregate, with a debt-free-date marker, `6wk.5`) and the debt-paydown **scenario** compare
  (ADR 0036 `debt_payoff`).
- **Investments sub-view** hosts holdings + (later) the growth scenario.
- **Net-worth timeline** spans all roles (liquid + investments − debt); it lives at the
  **Accounts surface level** (above the sub-views) since it is inherently cross-role, with the
  per-role contributions visible in each sub-view.
- **Dashboard** keeps only **summary widgets that link into** these sub-views (a net-worth
  number, a "next debt payment" tile) — a glance + a jump, not the analytics home. This
  satisfies §18.2's intent without making the dashboard the primary surface; dashboard
  *profiles* (§18.3 / `mh93`) remain a separate, later personalization concern, not blocked by
  or blocking this.

### 3. In-shell sub-navigation, not new routes (yet)

Per ADR 0020, a router (TanStack Router) is **deferred**; the app navigates in-shell. The
Liquid/Investments/Debt partition is therefore an **in-shell sub-navigation within the Accounts
view** (tabs/segments rendered by the existing shell), **not** new URL routes. If/when ADR 0020
adopts routing, these sub-views are the natural route boundaries — the component partition
chosen here maps cleanly to routes later, so this is forward-compatible, not a dead end.

### 4. The non-advice scan follows the copy here (ADR 0018)

The debt/investment views render descriptive projections (debt-free dates, balances, what-would
-it-take band math). Per ADR 0018 + its `915.1` addendum, `copy-review.test.ts` **extends its
scanned-directory list to `src/accounts`** (the new home) so the advice-phrase boundary is
enforced where this copy lands — otherwise relocating financial copy out of `src/future-cash` /
`src/dashboard` would silently drop it from the scan.

## Consequences

- `4d8.4` (the Liquid/Credit/Investment account grouping) realizes as the **sub-view partition
  of the Accounts shell**, not in-list grouped headers nor separate sections; its acceptance is
  updated to the named `AccountsView` sub-navigation.
- `6wk.5` (burndown / net-worth viz) builds against a known home (Debt sub-view + Accounts-level
  net-worth), unblocking it.
- No router work is pulled forward; the partition is a forward-compatible component boundary.

## Alternatives considered

- **Three top-level sections** (§18.1 / `sbm3`). Rejected — fragments "my accounts" across the
  nav; the owner prefers one Accounts home navigated by role.
- **Dashboard as the analytics home** (§18.2/§18.3). Rejected as primary — the dashboard is a
  glance surface; deep per-account analytics belong with the accounts. Summary widgets that
  *link in* are kept.
- **New routes now.** Rejected — ADR 0020 defers routing; in-shell sub-nav avoids pulling that
  forward while staying route-mappable later.

## Addendum (2026-07-06, personal-cfo-4d8.23.6/.7/.8): the sub-nav is dissolved

Owner dogfooding (2026-07-06) on the reshaped Accounts surface (Claude Design `Accounts.dc.html`,
ADR 0044) found the in-shell sub-navigation (§1/§3) redundant and confusing. The account model is
now first-class on a **single Accounts surface** (a two-column Assets | Liabilities balance sheet
with a net-worth summary, ADR 0044) and per-account debt terms are captured **in the account
editor drawer** (ADR 0044 §, `4d8.22.5`), so the "Debt" and "Update balances" tabs no longer earn
their own navigation. This addendum **supersedes §1 (Liquid/Investments/Debt sub-views) and §3
(in-shell sub-navigation)** and refines §2:

- **No sub-navigation.** The Accounts surface has no tab strip. The two-column Assets | Liabilities
  view (grouped into per-kind accordion **sections** with icons — Cash, Investments, Property &
  vehicles | Credit cards, Loans — each with its own filter/sort and a per-row edit + archive
  control) is the whole surface.
- **Balance updates are an inline mode**, not a tab: a toggle on the Accounts surface flips the
  list into mass balance-entry (`BatchBalanceUpdate`) in place.
- **Per-account debt terms live in the editor drawer** (not a Debt sub-view) — APR, statement/due
  days, minimum rule, paying source, etc.
- **Cross-account debt analytics are rehomed onto the Accounts surface, not a tab.** The
  loan-double-count **warning** (`LoanDoubleCountWarning`) surfaces inline at the top of the list
  (it is a data-integrity alert about the accounts you are looking at). The **card statement
  forecasts** (`CreditCardsView`) and the **debt-payoff comparison** (`DebtPayoffCompare`) move
  into a single collapsible **"Debt insights"** section at the bottom of the Accounts surface
  (collapsed by default). Their forward-looking nature makes the Future Cash surface a plausible
  longer-term home; relocating them there is a candidate follow-up, tracked separately, and is out
  of scope for this change (which only dissolves the sub-nav without losing the analytics).

The role partition (§ / ADR 0028) still drives the grouping — it is now expressed as sections
within the two columns rather than as top-level tabs.

## Relates to

ADR 0020 (routing deferral), ADR 0028 (role partition), ADR 0035/0036 (the debt analytics +
scenarios these views host), ADR 0044 (the account-model reshape this IA now follows),
ADR 0018 + `915.1` (copy-review scan extension).
