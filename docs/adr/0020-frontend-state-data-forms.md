# ADR 0020: Frontend state, data-fetching, and forms architecture

- **Status:** Accepted
- **Date:** 2026-06-20
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-otlv`](../../.beads/issues.jsonl)
- **Related plan sections:** §3.1
- **Supersedes:** None

## Context

The plan (§3.1) specifies the frontend stack: TanStack Router (or an equivalent
typed router), **TanStack Query** for frontend-to-Rust command-result caching,
Zustand/Jotai for small global UI state, and **React Hook Form + Zod** for forms
and runtime validation. Bead `personal-cfo-dily` ("React + TypeScript frontend
shell") restates that list as its acceptance criteria.

What actually shipped — the `x99h` design foundation plus the vault and feature
UIs (accounts, transactions, income, bills, dashboard) — adopted **none** of the
state/router/forms libraries:

- **Data fetching:** each screen has a hand-rolled `useX` hook (`useAccounts`,
  `useBills`, `useDashboard`, …) that calls a typed `commands.*`, holds its own
  `loading`/`error`/`refresh()` in `useState`, and re-fetches manually after a
  mutation. `UnlockedHome` even *remounts* tabs to force a refetch — a workaround
  for having no shared cache.
- **Forms:** plain `useState` fields with ad-hoc validation (`dollarsToMinorUnits`,
  `amountInvalid`, …).
- **Global state:** a small React Context (`useVault`) for vault status.
- **Routing:** `App.tsx` switches on vault lifecycle state; `UnlockedHome` uses a
  `useState` tab — no router.

There is **no ADR and no bead** governing this layer, and no recorded rationale
for skipping the planned libraries. That absence is *why* the layer drifted —
this ADR closes the governance gap. Notably, **ADR 0003 (Accepted) already names
"TanStack Query caches … cleared on lock"** as the frontend cache mechanism, so
the current hand-rolled approach also diverges from an accepted ADR, not just the
plan.

## Decision

We **adopt** the two libraries that earn their place at current scope and
**defer** the two that do not, with explicit rationale for each.

### Adopt: TanStack Query (server / cache state)

All reads that cross the IPC boundary go through TanStack Query.

- One query hook per read; a stable, typed query key per command + args.
- Mutations (create/update commands) **invalidate** the affected query keys
  instead of bespoke `refresh()` callbacks; tab remounts for refetch are removed.
- The `QueryClient` cache is **cleared on vault lock** — satisfying ADR 0003's
  "the cache is cleared on lock" requirement structurally.
- Loading/error/empty come from the query state, rendered through the shared
  state primitives (see the state-design-system beads), not re-implemented per hook.

*Why:* it replaces a partial, inconsistent, per-hook reimplementation of caching,
dedup, invalidation, and request state with the mechanism ADR 0003 already
assumes — and it pays off precisely as data is shared across screens (the accounts
list already feeds Accounts, the dashboard's liquid cash, and income/bill deposit
pickers).

### Adopt: React Hook Form + Zod (forms / validation)

All forms use React Hook Form with a **Zod schema per form**.

- The Zod schema is the single source of UX-time validation (amount > 0, required
  fields, date format, currency match). **Rust stays authoritative** (ADR 0003):
  Zod is for fast, friendly client feedback, never the security boundary.
- RHF owns field state, dirty/touched tracking, and submission, replacing the
  hand-rolled `useState` + manual-flag pattern.

*Why:* validation is currently duplicated and ad-hoc; a typed schema per form is
declarative, testable, and the plan's explicit "zod on the frontend" intent.

### Defer: TanStack Router

The app is a single window whose navigation is (1) vault-lifecycle routing in
`App.tsx` and (2) in-shell section tabs. It is not URL- or deep-link-driven. A
full router adds a route tree and history model the app has not earned. The plan
allows "**or equivalent typed router**"; the explicit state-machine + tab approach
is that equivalent for now.

*Revisit when:* deep-linking, many nested screens, or back/forward semantics are
needed (e.g. a settings area with sub-routes, or restoring a deep view on unlock).

### Defer: Zustand / Jotai

The only cross-cutting client state is vault status, held in a small typed React
Context (`useVault`). The plan says "keep [global UI state] small and explicit" —
a single-purpose Context *is* that, and is simpler than introducing a store.

*Revisit when:* genuinely global, multi-consumer UI state appears (command
palette, notification center, multi-panel layout, privacy-mode toggle).

### Conventions that follow (summarized; full doc: `docs/agent/FRONTEND.md`)

- IPC only through generated `commands.*` (never raw `invoke`) — ADR 0003.
- One TanStack Query hook per read; mutations invalidate their keys.
- Forms: React Hook Form + a Zod schema; Rust re-validates.
- Money/dates rendered via `lib/format` (`formatMoney`, `formatIsoDate`, …).
- Loading / error / empty / success use shared primitives, not per-screen markup.

## Consequences

### Positive

- A shared cache + invalidation removes the per-hook boilerplate and the
  remount-to-refetch hack; cross-screen data stays consistent.
- Declarative, typed form validation; one place to change a rule.
- ADR 0003's cache-clear-on-lock becomes real instead of aspirational.
- The frontend has a recorded decision, so the next screen can't silently re-drift.

### Negative

- A refactor of the existing ~6 hooks and ~5 forms (small now — that is the point;
  it only grows).
- Two new frontend dependencies (TanStack Query, RHF + Zod) to vet for bundle size
  and the no-network/CSP posture (both are pure JS, no network — compatible).
- A modest learning/convention cost for consistency.

## Rejected alternatives

- **Keep the hand-rolled approach.** ✗ The status quo: inconsistent request state,
  no shared cache, duplicated validation — the drift this ADR exists to correct.
- **Adopt all four planned libraries now.** ✗ Router and a store are unearned at
  current scope; adopting them only because the plan listed them is cargo-culting
  and adds complexity without a problem to solve.
- **A different data layer (e.g. SWR, RTK Query).** ✗ TanStack Query is the plan's
  choice and is already baked into ADR 0003; no reason to diverge.

## Revisit if

- Navigation becomes route/deep-link driven → adopt a typed router.
- Cross-cutting UI state outgrows a single Context → adopt Zustand/Jotai.

## Implementation notes

- Migration is **incremental and mechanical**, per hook and per form; the typed
  bindings and `describeIpcError` mapping stay as-is.
- The `QueryClient` must subscribe to vault-lock to clear the cache.
- This ADR is a **decision only**; the code migration is tracked as a separate
  implementation bead, and `dily` / `sbm3` are realigned to this decision rather
  than the original four-library AC.

## Linked beads

- `personal-cfo-otlv` (this ADR)
- `personal-cfo-dily` (React + TypeScript frontend shell — AC realigned here)
- `personal-cfo-sbm3` (App shell + main navigation)
- `personal-cfo-1al` (ADR 0003: trust boundary — TanStack Query cache, zod-for-UX)
- `personal-cfo-40t` (typed IPC command pattern)
- `personal-cfo-x99h` (frontend design foundation)
- `personal-cfo-027s` / `-mm7a` / `-tzds` / `-x3o0` (error / loading / empty /
  success state design systems — the shared primitives referenced above)
