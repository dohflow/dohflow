# ADR 0050: Forecast uncertainty model — localized variance propagation

- **Status:** Accepted
- **Date:** 2026-07-13
- **Deciders:** Project owner (2026-07-13 dogfooding feedback)
- **Beads:** [`personal-cfo-4d8.27.5.7.1`](../../.beads/issues.jsonl) (this ADR); implemented by
  `4d8.27.5.7.2` (per-account cash-account cone), `4d8.27.5.7.3` (card variable-spend lump
  injection), `4d8.27.5.7.4` (per-account spend-account band view); interim indicator half in
  `4d8.27.1.2`. Reconciles `9h1s`, `e596`, `7fxd`, `nxgx`.
- **Builds on:** ADR 0026 (§13/§13a Forecast Readiness + the Layer-2 band), ADR 0038
  (ordinary/extraordinary spend classification), ADR 0039 (card statement + payment model),
  ADR 0027 (additive balance / assertion-anchored starting balance).

## Context

The Future Cash forecast draws a **deterministic Layer-1 line** (known income + recurring
bills + transfers + card/loan payments) and, once ≥6 months of categorized variable spend
exist, overlays a **Layer-2 uncertainty band**.

As shipped, that band is:

- **Aggregate-only** — `apply_layer2_spend` runs only from the aggregate `compute()`
  (`forecast/aggregate.rs`); every per-account and per-tier series carries `Band::point`
  (zero width).
- **Liquid-only** — the aggregate history query filters `cashflow_role = 'liquid_cash'`, so
  **credit-card variable spend is excluded** (it is meant to reach cash via the card
  *payment* event, ADR 0039 §2).
- A **uniform daily smear** — `widen_with_spend` (`forecast-engine/src/layer2.rs`) walks days
  forward adding `band.pXX / days_in_month` for every learned category into a running
  cumulative band. It is a smooth cone but not localized to where variability actually
  enters.

Owner feedback (2026-07-13): the band should be **variance propagated from localized
sources** and should be a **per-account** feature first (the primary example: viewing a
single Venture X card — its history behind, its known recurring charges forward, and a band
telling you the range its balance may fall within over time). The key insight: **credit-card
variable spend does not hit cash when swiped — it collapses into a single uncertain lump at
each future payment date** (how big will the statement be?), which is exactly what the
statement estimator's walk-forward MAPE already measures. A closed statement with a locked
payment philosophy is *known* → no uncertainty there; the further out, the more unclosed
cycles and unknown discretionary spend → the wider the cone, and it **compounds** forward.

## Decision

1. **The band is variance propagated from localized injection sources, compounding forward.**
   The variance of the cash balance at day *T* is the accumulation of the variances of every
   variable flow that has occurred by *T*. Variance is **monotonically non-decreasing**; a
   known/closed amount is a **deterministic step** (changes the level, adds no width).

2. **Two injection kinds, each contributing a discretionary dollar's variance exactly once
   (no double-count):**
   - **Cash-account discretionary spend** (debit / checking / cash — liquid accounts) injects
     **continuously at spend time**, sized per-account per-category from the existing
     `SpendModel` Student-t half-width (`layer2.rs`).
   - **Credit-card variable spend** injects **nothing at swipe** and a **single lump at each
     future payment date**, sized from the statement estimator's walk-forward MAPE
     (`card_statement_estimator_v1`) applied to the **stochastic (variable) component** of that
     payment — not the whole payment (the carried balance + known bills + interest + any
     statement override are near-deterministic). A **closed** statement with locked philosophy
     (`statement_is_actual` / `close_date ≤ today`) injects **~0**.

3. **No-double-count invariant** (testable; generalizes the current `liquid_cash` filter into
   a paying-instrument partition): each discretionary posting contributes variance **either**
   at its card's payment date (if card-charged) **or** at spend time (if paid from a cash
   account), never both.

4. **Per-account is the primary home; the aggregate is the composite.** Each spend account
   carries its own cone (realized history behind via `cf-history`, the deterministic recurring
   line forward, a widening cone from its own variability). The aggregate/tier band is
   **derived by composing the per-account cones** (`roll_up_groups` / `add_band` already sum
   per-account bands), which removes the liquid-only restriction — card variance now lives on
   the card account and flows into cash at the payment date.

5. **Method: Monte Carlo path sampling.** Sample *N* spend/statement paths through the horizon
   and read per-day percentiles. MC natively handles the statement→payment nonlinearity
   (minimum-payment floors, revolving via `project_revolving`), the lumped injections, and
   cross-account correlation (joint sampling). To preserve `forecast-engine`'s reproducible /
   byte-identical contract, the RNG is **seeded from the input snapshot** (`eqfw`) so a run is
   deterministic given its inputs. **Analytic variance addition** (quadrature: `Var +=
   (MAPE·variable)²` at each payment date, per-category σ² continuously) is documented as the
   fallback if MC cannot meet the forecast performance budget on the golden fixture; it
   composes with the shipped Student-t widening but assumes independence and does not capture
   the statement→payment coupling. *This method choice is the ratification point for this ADR —
   MC is chosen; see Alternatives.*

6. **Known/closed amounts add a deterministic step** (level change, no width). Variance is
   monotonically non-decreasing across the horizon.

**Refinement (2026-07-13, phasing).** MC is applied where non-linearity or correlation actually
matters — the card-payment lump (statement→payment with min-payment floors / revolving) and the
correlated aggregate composite. The per-account **cash** cone is built with the existing analytic
Student-t widening (`widen_with_spend`), which is provably equivalent for a single account's own
continuous spend (central limit theorem) and avoids introducing an RNG into the pure
`forecast-engine` crate — its CI purity boundary forbids any RNG/IO crate, so MC will use a
bespoke seeded PRNG when it lands. Consequently the **aggregate = composite** unification
(Decision 4) ships with the MC card-lump work, where joint sampling composes correctly; the
analytic cash-cone step keeps the calibrated pooled aggregate band (Decision 4's composite would,
done analytically, over-widen it because per-account models are data-thin and band summation is
comonotonic). The §12 reconciliation invariant is a Layer-1 property; per-account cones compose
statistically, not by linear P50 addition.

## Reuse (largely intact)

`Band` / `BandDto` (already p10/p50/p90 per-day, per-account, per-group — currently collapsed
off the aggregate); `SpendModel` + `prediction_half_width` (Student-t, shrinks with evidence);
`estimate_card_new_charges` + the `card_statement_estimator_v1` MAPE
(`forecast_backtest_results`); `build_attribution`'s card-payment routing to the paying liquid
account at the payment date; `roll_up_groups` / `add_band`; `statement_is_actual`;
`read_variable_spend_history` (already account-scoped-capable); `eqfw` model registry +
`46jq` actualization (the source of the walk-forward error); `tu2i` band shape.

## Build / change

- Move widening out of the aggregate-only path into per-account `run_series`
  (`account_series.rs`): each `AccountSeriesView` accumulates a monotonically widening cone
  from localized injections.
- Add a **reader for the `card_statement_estimator_v1` MAPE** row (`forecast_backtest.rs`
  `latest_mape` is hard-scoped to `layer1_deterministic`; the card metric is write-only outside
  tests today).
- **Decompose the card payment** into deterministic (carried balance + known bills + interest +
  override) vs stochastic (`projected_variable_minor`) so width applies only to the estimated
  part; map charge-error → payment-error through the philosophy (full-payer ≈ 1:1 on the
  variable part; minimum/fixed dampen it).
- **Partition discretionary spend by paying instrument** (the de-double-count routing).
- Refit `7fxd` to validate **per-account cone coverage** (variance calibration), not just P50
  MAPE.

## Consequences

- **Readiness re-converges on a card-inclusive spend-history predicate** (ADR 0026 §13a
  addendum): because the band now consumes card spend, the `spending_history` factor counting
  card spend is a consequence, not a divergence. The interim indicator fix (`4d8.27.1.2`) ships
  first as honesty; the divergence decision is folded here.
- `nxgx`'s "band = liquid-only" premise is refined; the per-account cone is a new unlockable
  capability (reuses `egon`).
- The composite aggregate must equal the per-account roll-up — the reconciliation invariant is
  extended from levels to variances.

## Alternatives considered

- **Analytic variance addition (quadrature).** Cheaper and deterministic, and it composes with
  the shipped Student-t widening. Rejected as the primary method because it assumes independence
  and cannot represent the statement→payment coupling (a big month raises the statement, which
  is paid as one lump subject to min-payment floors and revolving). Retained as the documented
  fallback if MC misses the performance budget.
- **Keep the uniform aggregate overlay.** Rejected: it is not localized (it invents width where
  amounts are known, e.g. a closed statement), it cannot be per-account, and it structurally
  excludes card spend.

## Open items (ratify on the build PRs)

- **Per-card vs shared MAPE width.** The current MAPE is a single aggregate across all cards and
  lead times. Start with the shared relative width; bucket walk-forward pairs per card / per
  lead-time as a refinement for a truer per-account cone.
- Exact MAPE→payment-date σ mapping through each philosophy.

## Invariants to test

Monotonic non-decreasing variance; closed statement + locked philosophy → ~0 injection;
no-double-count (a card-charged dollar never injects at spend time); per-account cones compose
to the aggregate cone under MC; a deterministic step (known amount) adds level, not width.
