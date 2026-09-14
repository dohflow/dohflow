# ADR 0049: Sidebar information architecture — grouped sections, Cash Flow, and top-level Debt

- **Status:** Accepted
- **Date:** 2026-07-20
- **Deciders:** Project owner (2026-07-11 mock review; 2026-07-12 IA choices; 2026-07-13 dogfooding feedback)
- **Beads:** [`personal-cfo-4d8.27.4.1`](../../.beads/issues.jsonl) (this ADR + its
  implementation); subsumes `4d8.26` (the deferred sidebar reorg); reverses `xdbm` (the unified
  Transactions hub) and rescopes `4d8.24.8` (cross-list multi-select → per-surface); reserves
  placement for
  `4d8.27.9.x` (Debt page) and `4d8.27.6.1` (Scenarios tab).
- **Builds on / supersedes:** ADR 0037 (cross-account analytics IA — this supersedes its
  "not three separate top-level sections" stance *for Debt*, and its 2026-07-06 addendum's
  placement of Debt insights inside Accounts), ADR 0020 (routing deferral — still in-shell),
  ADR 0031 (UI quality), ADR 0018 (non-advice copy scan).

## Context

The sidebar grew as a **flat list** of eight destinations, and two of the owner's dogfooding
observations converge on it:

1. **2026-07-11** (reviewing the Card Review design mock): *"Claude Design actually did a really
   good job breaking the Money Inbox out from Transactions and having it in a separate Review
   section in the menu bar. The menu bar re-organization to something like this should be
   something we do as well… this looks much more user friendly and cleaner."*
2. **2026-07-13**: rebrand Future Cash to **Cash Flow** (past *and* future), pull **Debt
   insights** out of Accounts into its own page, and give **Scenario planning** its own tab.

Today `TransactionsHub` (ADR-less, bead `xdbm`) composes Money Inbox + Activity + Recurring
(Bills + Recurring Transfers) into one screen, and Bills/Recurring/Money Inbox have no top-level
home. ADR 0037's addendum put Debt insights in a collapsed section at the bottom of Accounts —
while itself noting *"relocating them there is a candidate follow-up, tracked separately."* This
ADR settles the whole navigation shape once, so the Scenario and Debt arcs build onto a stable
base instead of each re-litigating the nav (AGENTS.md §1A).

## Decision

### 1. Three grouped nav sections, plus a pinned utility group

The sidebar renders **labelled groups** instead of one flat list:

| Group | Destinations |
| --- | --- |
| **Overview** | Dashboard · Accounts · *Debt (reserved)* · Transactions |
| **Planning** | Cash Flow · *Scenarios (reserved)* · Bills · Recurring Transfers · Income |
| **Review** | Money Inbox (pending-count badge) · Categories |
| *(pinned bottom)* | Search (action) · Backup · Settings · Lock |

Groups are visual + semantic only — navigation stays **in-shell** (ADR 0020's router deferral is
unchanged); a group label is not itself selectable.

### 2. "Transactions" is the Activity list; the unified hub dissolves

`TransactionsHub` (bead `xdbm`) is **reversed**. Its four tenants become their own destinations:
**Transactions** = the Activity list only (owner's explicit choice, 2026-07-12), **Money Inbox**,
**Bills** (with its suggested-recurring panel), and **Recurring Transfers**. The hub folded them
together because they "belong together"; a year of use showed the opposite — the hub buried the
review queue and gave bills no findable home. Each surface already supports standalone rendering,
so this is a composition change, not a rewrite.

### 3. Cross-list multi-select becomes **per-surface** multi-select

`4d8.24.8`'s *union* selection across Money Inbox + Activity existed **only** because the hub put
both lists on one screen. With them on separate destinations there is no cross-list to unify, so
the union is retired (the owner-approved 2026-07-12 call).

What is **not** retired is the selection machinery itself. Each surface now owns its own
`useTransactionSelection` instance, supplied by the shell. This matters concretely: the Money
Inbox's **"Select all N in inbox"** bulk control (`4d8.25.16`) and its register-visible-rows
freshness handling are implemented on the *controlled* path only — deleting the hook outright
would have silently dropped a shipped feature the owner asked for. Scoping the hook per surface
keeps every bulk behaviour intact on both surfaces while removing only the cross-list union, and
leaves the two large list components untouched (no branch surgery in the highest-traffic files).

### 4. Future Cash → **Cash Flow**

The section is renamed in the UI (route token `future-cash` → `cash-flow`, label and headings).
The name now matches what it shows — realized history *and* projection (ADR 0050 / ADR 0026 §13a)
— rather than only the forward half.

**Storage and wire identifiers are deliberately NOT renamed.** The IPC commands
(`future_cash_forecast`, `future_cash_by_account`), the settings-KV key behind the series picker
(`future_cash_series_selection`), and the `src/future-cash/` source directory keep their names: a
UI label is not a schema, and renaming persisted keys would need a migration for zero user value.
The label is the product surface; the identifiers are implementation detail.

### 5. Debt is promoted to its own top-level destination

**Supersedes** ADR 0037 §1's "not three separate top-level sections" *for Debt*, and its
2026-07-06 addendum's placement of Debt insights inside the Accounts surface. Debt earns a
top-level page because the owner wants per-account depth (single **and** multi-account selection,
per-debt-type visualizations, a scoped transaction log, payoff tools) that cannot live in a
collapsed section without dominating Accounts. Accounts remains the canonical home for *account
identity and balances*; Debt is the home for *debt analysis*. Investments are **not** promoted by
this ADR — no owner need has been expressed, and promoting one role does not imply promoting all.

### 6. Reserved destinations land with their pages

Debt and Scenarios are **reserved placements**, not nav entries yet: adding a sidebar item that
routes nowhere is a dead link. Their entries ship with their surfaces (`4d8.27.9.2`,
`4d8.27.6.1`) into the slots defined above, with no further IA decision required.

## Consequences

- The Money Inbox becomes findable (its own destination with a pending-count badge) rather than a
  banner on a screen the user visits for another reason.
- Bills and Recurring Transfers gain findable homes; "go to Transactions → Recurring" copy and any
  hub-relative navigation must be updated (no orphaned references).
- The cross-list *union* is gone (accepted per §3); every per-surface bulk behaviour — including
  the inbox's Select-all — is preserved.
- The Scenario tab and Debt page arcs are unblocked with their placement pre-settled.
- `4d8.26` is subsumed by this ADR and its implementation; it closes with that reason.
- Nav-shape tests, the `Tab` union, Cmd-K search entries, and onboarding copy follow the rename.

## Alternatives considered

- **Keep the flat list, rename only.** Rejected: it leaves the Money Inbox buried and gives Bills
  no home — the owner's actual complaint.
- **Keep the unified hub and add groups around it.** Rejected: the hub *is* what buries the
  review queue; grouping the sidebar without dissolving it treats the symptom.
- **Rename the `src/future-cash/` directory and the KV/IPC identifiers to match.** Rejected as
  churn: a large diff plus a settings migration for no user-visible gain (§4).
- **Promote Investments alongside Debt.** Deferred: no expressed need; revisit if the investments
  arc grows analytics of its own.
