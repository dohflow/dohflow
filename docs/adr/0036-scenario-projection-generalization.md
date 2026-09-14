# ADR 0036: Scenario / projection generalization across cash, debt, and investments

- **Status:** Proposed
- **Date:** 2026-06-30
- **Deciders:** Project owner
- **Beads:** [`personal-cfo-5ie.6`](../../.beads/issues.jsonl) (this ADR); gates `od07`
  (debt-paydown scenarios) and the investment-growth what-if
- **Builds on:** ADR 0026 (forecast architecture — §5 scenarios as assumption-event overlays,
  §3 persisted runs, the `6zep` override engine), ADR 0035 (debt-payment model — debt balance
  trajectories), ADR 0018 (non-advice)

## Context

ADR 0026 §5 defines a **scenario** narrowly: a named overlay of assumption events on the
**cash** forecast, composed over base by the `6zep` engine, compared against base in the
`ScenarioBar`, and (via `0mg`) pinned to a `forecast_run`. The debt arc (`od07`) wants
**debt-paydown** what-ifs ("avalanche vs snowball vs +$200/mo — debt-free date and total
interest per plan"), and the investments arc will later want **growth** what-ifs. Neither is a
cash `forecast_run`; both are projections over a *different* quantity (a liability balance
burning down, an asset balance compounding up). Building either as a one-off surface would
duplicate the overlay/compare/render machinery and diverge. This ADR decides whether and how
"scenario" generalizes — a decision both downstream beads implicitly make, so it must be fixed
first (AGENTS.md §1A).

## Decision

### 1. Generalize to a polymorphic **projection run** with a `kind`

A scenario becomes a **projection run** parameterized by `kind ∈ {cash, debt_payoff,
investment_growth}`, reusing the *same* `6zep` assumption-event-overlay engine, the same
compare-vs-base contract, and the same `ScenarioBar`/compare UI — only the **projected
quantity** and the **base series** differ per kind:

- **`cash`** — the existing Future Cash forecast (liquid balance over the horizon). Unchanged.
- **`debt_payoff`** — a **liability-balance burndown**: starting owed balance projected down
  under the repayment model (ADR 0035 §1/§4 — payments reduce, finance charges add), with the
  overlay expressing the what-if (an extra fixed payment, a different `repayment_philosophy`, a
  paydown-order strategy across multiple debts). The compared outputs are **debt-free date**
  and **total interest** per plan.
- **`investment_growth`** — an **asset-balance projection** under contributions + a return
  assumption. **Reserved, not in MVP scope** (see §3).

The overlay vocabulary is shared (an assumption event is "an extra $200/mo against Card A from
date D"); the *engine that folds it* is shared; the *series it folds over* is per-kind.

### 2. Non-cash projections are plans, not forecasts — they are **not actualized**

The actualization + backtest machinery (§9/§17/§18, `46jq`) scores a **persisted cash
forecast** against realized transactions. A `debt_payoff` or `investment_growth` projection is
a **hypothetical plan** the user is comparing, not a prediction of what will happen — so it is
explicitly **not** actualized and does **not** persist `forecast_actuals` / quality scores. The
debt *account's* realized balance is tracked by the ledger as normal; the *scenario* is a
what-if rendered on demand, discarded or saved as a named plan, never scored "right/wrong."
This keeps the actualization contract clean (it means one thing: "was the cash forecast
accurate") and avoids inventing a scoring story for hypotheticals.

### 3. Scope: `cash` + `debt_payoff` now; `investment_growth` reserved + deferred

`cash` exists; `debt_payoff` is in scope (`od07`, on the ADR 0035 debt model). The
`investment_growth` **kind is reserved in the abstraction** so the surface and engine admit it
later without rework, but the kind itself is **deferred** — investment return modeling is
post-MVP (consistent with the `9h0` / plan §16.1 deferral), and a growth projection needs a
return-assumption model this ADR does not decide. Building the abstraction now (with two live
kinds) is what lets the third slot in cheaply.

### 4. Non-advice (ADR 0018)

Scenario comparison is **descriptive**: each plan's debt-free date and total interest are
stated side-by-side, **none labelled "best" / "recommended" / "you should"**. The compare view
ranks by the user's chosen metric (e.g. soonest debt-free) as a neutral sort, not a
recommendation. Copy is governed by the ADR 0018 boundary + its addendum (`915.1`).

## Consequences

- `od07` is rescoped from a one-shot payoff calculator to **named debt-payoff projection runs**
  reusing the cash scenario engine (no parallel surface).
- The persisted-run schema (`0mg` `scenarios`) gains a `kind` discriminator; `base_run_id`
  stays meaningful for `cash`, and is `NULL`/kind-specific for the others (they pin to the debt
  account set + repayment model, not a `forecast_run`).
- Investment growth, when it arrives, is a new `kind` + a return model — not a new surface.

## Alternatives considered

- **Parallel debt/investment surfaces.** Rejected — duplicates the overlay/compare/render
  machinery and diverges from the cash scenario UX users already learn.
- **Actualizing debt/investment projections.** Rejected — they are hypotheticals; scoring a
  what-if against reality is meaningless and would muddy the actualization contract.
- **Building `investment_growth` now.** Deferred — needs a return-assumption model out of MVP
  scope; the abstraction reserves the slot.

## Addendum (2026-07-01, personal-cfo-6wk.19): the `recurring_debt_payment` overlay kind

§1 names the shared overlay vocabulary example "an extra $200/mo against Card A from date D".
That overlay is now a concrete assumption kind, `recurring_debt_payment`, so a `debt_payoff`
plan's extra monthly payment can render on the **cash** forecast (personal-cfo-6wk.15): the
selected plan's extra becomes a scenario-scoped overlay that projects a recurring liquid
**outflow** (ADR 0035 §3 debt-payment leg), and the existing compare-vs-base machinery shows its
effect on projected liquid cash — no parallel surface.

- **Params** (hand-built JSON, serde-parsed on read, matching the other kinds): `amount_minor`
  (positive magnitude, projected negated), `currency`, `anchor_date` (monthly day-of-month
  anchor), optional `end_date`, `label`.
- **Free-standing** (no `target_entity_id`) — it *adds* a flow rather than modifying a base
  entity, so it folds via its own read consumer (`recurring_debt`) + a dedicated collector, like
  the one-time-event addition path, not the entity-override path.
- **Scenario-scoped** by the usual `scenario_id IS NULL OR = ?1` gate: the base cash forecast is
  unaffected unless a base-level overlay is recorded. Migration v31 widens the immutable v10
  `kind` CHECK (append-only; the table is rebuilt).

This slice ships the overlay primitive + forecast application only; turning a selected plan into
a named scenario and the Future Cash UI are the remaining `6wk.15` slices.

## Relates to

ADR 0026 (§5 scenarios, the `6zep` engine this generalizes), ADR 0035 (the debt model a
`debt_payoff` projection runs over), ADR 0018 + `915.1` (descriptive compare copy).
