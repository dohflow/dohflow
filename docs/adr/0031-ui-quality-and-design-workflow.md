# ADR 0031: UI-quality discipline and the Claude Design workflow

- **Status:** Accepted
- **Date:** 2026-06-27
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-6s8t`](../../.beads/issues.jsonl)
- **Related:** ADR 0020 (frontend state / data / forms), `personal-cfo-x99h` (design tokens / `globals.css`), [`docs/design-system/`](../design-system/) (the portable kit), [`docs/agent/FRONTEND.md`](../agent/FRONTEND.md)

## Context

Dogfooding round 2 (2026-06-27 feedback) reported that functionality is landing well
but the UI lags it and "doesn't feel like we're following the design guidelines or
using the shadcn set we agreed upon." The maintainer is getting blocked on *testing*
features because the surfaces are rough.

Two structural facts explain the gap:

1. **Only four trivial UI primitives exist in code** — `Button`, `Card`, `Input`,
   `Label` (`apps/desktop/src/components/ui/`). Every richer surface — the Future Cash
   chart, the projected-activity table, every modal — is **hand-rolled per view**.
   There is no shared, themed chart / table / dialog / form, so quality and
   consistency vary screen to screen. [`recipes.md`](../design-system/recipes.md)
   documents how these *should* look, but they are not yet code.
2. There is a **portable design-system kit** ([`docs/design-system/`](../design-system/):
   `design-system.md`, `design-tokens.json`, `recipes.md`, `component-gallery.html`;
   tokens mirror `globals.css` per `x99h`) intended for mocking in **Claude Design** —
   but no agreed loop for *when* and *how* to use it, and `/design-sync` (the live sync
   into Claude Design) is gated behind interactive auth.

We need a durable decision on **how we build UI**: when to just build, when to mock
first, who drives Claude Design, and how we stop the quality gap from re-accumulating
between dogfooding rounds.

## Decision

### 1. shadcn primitives *in code* are the quality lever

Promote the `recipes.md` components — **interactive chart** (shadcn + Recharts),
**data table**, **dialog/modal**, **RHF + Zod form** — into real, owned components
under `apps/desktop/src/components/ui/`, themed entirely through the `globals.css`
tokens. New screens **compose these primitives** instead of hand-rolling. This is the
single highest-leverage fix for "doesn't feel like shadcn": consistency comes from
shared primitives, not from re-styling each view. Tracked by `personal-cfo-4d8.6`.

Token rule (from `recipes.md`): chart colors are **full hex** tokens, so `ChartConfig`
uses `color: "var(--chart-1)"`, **never** `hsl(var(--chart-1))`.

### 2. Three routing lanes for any UI work

- **Mechanical / correctness** (a layout bug, a wrong value, a missing state):
  **just fix it** — no mockup. E.g. the sidebar full-height bug; the forecast
  per-row running balance.
- **Compose-from-primitives** (a screen that is cards + the shared
  table/chart/dialog/form in standard layouts): **build directly** against
  `design-system.md` + the gallery; no Claude Design round-trip.
- **Net-new / complex screen** (a novel layout, dense information design, or a flow
  with real interaction-design questions — e.g. the Transactions overhaul, the
  duplicate-review panel, the projected-activity redesign): **mock in Claude Design
  first** via the loop in §3.

### 3. The Claude Design loop (and why the agent can't drive it)

When a bead needs a design pass the agent does **not** attempt to run Claude Design.
`/design-sync` requires interactive auth (`/design-login` needs a tty), which a
headless agent session does not have. Instead:

1. The agent marks the bead **needs-design** and writes a **ready-to-paste Claude
   Design prompt** into the bead's `design` field: the screen's purpose, the data it
   shows, the required states (loading / empty / error / success), the component basis
   (shadcn + which primitives), the tokens, light + dark, and any interaction
   requirements — plus the instruction to paste `design-system.md` + `design-tokens.json`
   + a `component-gallery.html` screenshot as context.
2. The **maintainer** runs Claude Design with that prompt, iterates, and returns a
   screenshot / spec.
3. The agent **implements to match**, then drops the shipped screenshot into
   [`docs/design-system/screenshots/`](../design-system/screenshots/) (indexed in
   `design-system.md` §8) so the kit stays current.

### 4. `/design-sync` stays deferred until the kit has real components

`/design-sync`'s value scales with the number of real, reusable, themed components;
with four trivial ones it has little to sync. **Re-enable it once `recipes.md`'s four
primitives (chart, dialog/modal, data table, form) have gone from mockup to actual
code** in `components/ui/` (≈8–12 real components total). It runs only from an
interactive terminal (`/design-login` then `/design-sync`), never from an agent session.

### 5. UI-quality issues are tracked as beads, not absorbed silently

A quality gap — a chart that reads as a basic sparkline, a table that doesn't paginate,
an inconsistent surface — gets **its own bead** under the UX epic (`personal-cfo-4d8`),
the same as a functional gap. This is what stops the backlog from silently
re-accumulating between dogfooding rounds (the §15 continuous-dogfooding model in
`AGENTS.md`).

### 6. The design system evolves with the app

`design-system.md` §8 is the living loop: each shipped screen drops a screenshot,
recurring patterns get promoted into the spec + the gallery + (if a real component)
`components/ui/`. The kit is not a one-time artifact.

## Consequences

### Positive

- One consistent visual language: every new screen inherits shadcn quality from shared
  primitives instead of re-deriving it.
- A clear answer to "mock first or build first?" — routed by the three lanes, not
  decided ad hoc per screen.
- The agent and maintainer have explicit, non-overlapping roles in the design loop;
  nothing waits on a capability the agent doesn't have.
- Re-enabling `/design-sync` has a concrete trigger, not a vibe.

### Negative

- An up-front investment to build the four primitives before the screens that need them
  (sequenced as the first UI work, `4d8.6`).
- The Claude Design loop adds a round-trip for net-new screens — mitigated by scoping it
  to net-new/complex screens only; compose-from-primitives screens skip it.

## Rejected alternatives

- **Ignore design now, polish last.** ✗ Precisely what produced the R2 backlog and is
  now blocking the maintainer from testing. Rejected.
- **Perfect every screen as we go.** ✗ Too slow; over-designs simple
  compose-from-primitives surfaces. Rejected in favor of three-lane routing.
- **Agent drives Claude Design directly.** ✗ Not possible — `/design-login` is
  tty-gated; a headless agent has no interactive terminal. The bead + prompt loop is
  the workable substitute.
- **Keep hand-rolling per-view components.** ✗ The root cause of the inconsistency;
  rejected in favor of shared primitives.

## Revisit if

- The four primitives land and ≈8–12 real components exist → re-enable `/design-sync`
  (§4) and fold its output into this loop.
- A design tool with a non-interactive / API path becomes available to the agent →
  revisit §3 (the agent could then mock directly).

## Linked beads

- `personal-cfo-6s8t` (this ADR)
- `personal-cfo-4d8.6` (adopt the shadcn primitives into `components/ui/`)
- `personal-cfo-x99h` (design tokens / `globals.css` — source of truth)
- ADR 0020 (frontend state / data / forms — the RHF + Zod target the form primitive references)
