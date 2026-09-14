# ADR 0026: Forecast architecture (layered engine, reproducible pipeline, data-progressive activation)

- **Status:** Accepted
- **Date:** 2026-06-21
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-5ie`](../../.beads/issues.jsonl) (Future Cash epic)
- **Related plan sections:** §13 (forecasting), §13.1 two-tier, §13.2.2 readiness, §13.6 explainability, §13.8 chart, §13.10 backtest
- **Amends:** None (first forecast ADR)
- **Relates:** [ADR 0003](0003-trust-boundary.md) (typed explanation blocks), [ADR 0011](0011-hybrid-ledger-operation-log.md) (immutable events), [ADR 0018](0018-forecast-non-advice.md) (non-advice — *unwritten; mechanism established here*), [ADR 0021](0021-date-timezone-policy.md) (calendar/tz)

## Context

Future Cash is the product wedge (§5.1): the phase gates forbid connectors,
investments, or agents until forecasting is *useful and trustworthy*. Today only
**Layer 1** exists — a pure deterministic ledger (`personal-cfo-164u`,
`crates/forecast-engine`) that folds known income/bills over the starting balance.
It is **computed on demand** (`db-worker::forecast::compute`), **never persisted**,
and there is **no forecast ADR at all**. The most architecturally significant
subsystem in the product has zero recorded design.

The full vision (the `5ie` epic) is a layered system: deterministic → statistical
(variable spending → confidence bands) → behavioral (trend/change-point) → Monte
Carlo, plus scenarios, per-row explainability, a reproducibility/backtest spine,
and a Forecast Readiness score.

The owner's directive (2026-06-21): **do not ship a primitive chart and retrofit
the real thing later — build the infrastructure for the full vision now**, even
across many PRs. The hard reality in tension with that: the statistical layers
(L2–L4) model *variable spending*, which needs **months of categorized history**
to be meaningful — and [ADR 0018]'s non-advice rule forbids rendering precision we
haven't earned. You cannot make a trustworthy confidence band out of two weeks of
data.

The resolution, and the central idea of this ADR, is **data-progressive
activation**: build the *whole* layered architecture now — deterministic-driven —
and **gate each layer's output on a data-readiness score** so capabilities
*self-activate* as a vault matures. Nothing is primitive; nothing is retrofitted;
the only thing that "waits" is the *meaningfulness* of statistical output, which is
governed honestly by the gate.

## Decision

### 1. One band-shaped forecast row across all layers

Every layer emits the **same row shape**: a band — `P10 / P50 / P90` (minor units,
checked `Money`) — plus typed **provenance**. Deterministic Layer 1 sets
`P10 = P50 = P90` (a degenerate band = a line); higher layers widen it. Rows also
carry `computation_mode` (interactive vs batch) and an `assumption_basis`.

This single shape is the keystone: the **storage, IPC DTO, and chart are built once**
for bands and never rebuilt. The deterministic chart that ships first *is* the full
chart, rendering a collapsed band.

### 2. Two-tier compute (§13.1.1)

- **Interactive incremental** — on an edit, recompute only the affected
  `dirty_ranges` (target < 50 ms) for a responsive ledger.
- **Batch full-regeneration** — for persisted snapshots, backtests, and Monte
  Carlo. Rows record which mode produced them.

### 3. Reproducible, persisted pipeline (replaces on-demand compute)

A forecast is a **persisted run**: `forecast_runs` + `forecast_rows` (schema
`63t`, already migrated) plus an **input snapshot** and a **model registry**
(`personal-cfo-eqfw`). A run is exactly reproducible from
`(input_snapshot_id, model_versions, assumptions_hash, scenario_ids, seed,
code_version)`. This is the spine for backtesting, run-to-run diffs, and
explainability. Today's on-demand `compute` becomes the interactive tier feeding
this pipeline.

### 4. Assumption-event model — the input/scenario/explanation backbone (`5u2`)

Every input a forecast depends on is a **versioned, immutable assumption event**
(`forecast_assumption_events`): income amount/date, bill amount/date, card-payment
behavior, `minimum_cash_floor`, variable-spend override, one-time event, inflation,
scenario toggle, exclusion. **User edits never mutate forecast rows** (rows are
outputs) — they append assumption events (ADR 0011 shape). `forecast_dependency_edges`
link events → rows (powering explanation and incremental invalidation); a
`dirty_ranges` queue drives §2's interactive recompute.

**How the forecast honors events** (ratified 2026-06-22, personal-cfo-6zep). Beyond
*folding* additions, the projection *applies* override + exclusion events to base
income/bills. Each kind's `params_json` (hand-built on write, `serde_json` on read —
the `q6gh` shape):

- **addition** `one_time_event` → `{amount_minor, currency, date, label}`; folded as a
  `ManualOneOff` event (no target).
- **amount modification** `bill_amount` / `income_amount` → `{new_amount_minor,
  effective_date?, end_date?}`; substitutes the amount for the `target_entity_id` base
  entity's occurrences **within the window** `[effective_date, end_date]` (an absent
  `effective_date` opens the start; an absent `end_date` is open-ended — the legacy
  shape). Multiple amount modifications on one entity **compose over time** (see
  Composition); this lets a step-down schedule (e.g. a paycheck dropping across a leave,
  then returning to baseline) be expressed as a sequence of windowed overrides
  (personal-cfo-w6o9).
- **date modification** `bill_date` / `income_date` → `{new_anchor_date}`; shifts the
  target's schedule anchor.
- **removal** `exclusion` → `{effective_date?}`; skips the target's occurrences (from
  `effective_date` if set, else entirely).

`target_entity_id` names the base income source / recurring obligation. Composition:
the active **scenario layer composes over base**, and within an entity **amount
overrides compose by window** — each occurrence takes the amount of the override whose
`[effective_date, end_date]` window covers its date; where windows overlap the
later-created event wins; a date no window covers falls back to the base amount. (This
replaces an earlier latest-wins-*entirely* rule that silently dropped all but the last
amount change — personal-cfo-w6o9.) Date shifts and exclusions remain latest-wins. The
forecast loads these into a per-entity override map and consults it while expanding
occurrences — so the same machinery serves base overrides *and* scenario overlays.

### 5. Scenarios = assumption-event overlays

A scenario is a named set of assumption events layered over the base. Re-running
yields an alternative forecast. Because overlays are just events, **scenarios work
on deterministic data immediately** ("cut discretionary $200/mo", "delay rent a
week", "lose this income"). The chart gets a scenario selector.

Concretely (ratified 2026-06-21, reconciling the plan's original two-table sketch):
a `scenarios` table holds only the **named definition + lifecycle** (draft/active/
archived + the `base_run_id` it forks from). A scenario's events are ordinary
`assumption_events` with `scenario_id` set — there is **no separate
`scenario_events` table**. Running scenario X = base events (`scenario_id IS NULL`)
+ X's events, i.e. `active_assumption_events(Some(x))`; `eqfw`'s snapshot already
pins which scenarios a run applied via `scenario_overlay_ids`. This is what lets
scenarios inherit `5u2`'s explanation (`dependency_edges`), invalidation
(`dirty_ranges`), and undo (`supersede`) for free, and keeps one event model across
manual entries (`q6gh`), scenarios (`6zep`), and goals (`aoyl`).

### 6. Per-row explainability (`vkge`)

Each forecast row carries a **typed, structured** explanation — contributing
assumption events, dependency edges, and the row's `assumption_basis`. Rendered
from typed blocks only (ADR 0003: never raw HTML/Markdown). Layer 1 already emits
an `assumption_basis`; this surfaces it and grows as layers add their own
contributing factors.

### 7. Pluggable models behind a registry; ship baselines; develop on synthetic data

L2/L3/L4 are **versioned, swappable models** registered in the model registry
(`eqfw`). The pipeline, band shape, gate, and chart are **fixed**; the models
**evolve**. We ship **defensible baselines** — *not* final tuned models — e.g.
**L2 = empirical quantiles of historical daily discretionary net flow**. Better
models register later with **zero pipeline rework** and their own ADRs (§"model
decisions are deferred"). Registered so far: the per-card **statement new-charges
estimator** (ADR 0039, 2026-07-10 addendum §2 — signal-tiered, works without
transaction detail).

Crucially, models are **developed and validated against synthetic household data**
(`personal-cfo-9ujs`) + golden-fixture vaults, so the engine is *real and tested*
before any real user data exists. Synthetic data is therefore load-bearing
infrastructure, not a test nicety.

#### Addendum 2026-07-31 — category spend overrides adjust the model, not the event stream (`4d8.27.6.2`)

The `variable_spend_override` assumption kind (declared since `5u2`, unconsumed until now)
lets a user plan a change to one category's discretionary spend — "reduce Dining by
$200/month from August". It resolves against the **Layer-2 statistical draw**, never the
Layer-1 event stream.

That is forced, not stylistic. Discretionary spend has no representation in Layer 1:
`ForecastEvent` has no category field and the collectors are category-blind, by design —
Layer 1 is entered obligations, Layer 2 is everything else, and the split is what keeps
the two from double-counting. Emitting a deterministic `−$200` event for a dining cut
would therefore subtract dining twice: once in the fabricated event and again in the
unchanged band. It would also invent an obligation the household never entered, and put a
phantom row in Projected Activity.

The attachment point is exact: `SpendModel` is keyed by `(category, calendar-month)`,
where the category key is the category id the observations carry, so an override addresses
one bucket directly. The adjustment is applied per-day inside `widen_with_spend_adjusted`,
prorated across the month exactly as the band itself is, so a partial-month window needs
no special case.

A planned change is a **household** figure, but each account carries its own fitted
model, so the change is **apportioned across accounts** in proportion to how much of that
category each one actually spends (the largest share absorbs the integer remainder, so the
parts sum to exactly the stated figure). Handing every account the full delta would apply
the cut once per account, and the per-account chart would disagree with the aggregate
about what the very same plan does. An account that never spends in the category absorbs
none of it.

Three further properties are deliberate:

- **It moves the expected draw, not the spread.** Committing to spend $200 less does not
  by itself make the outcome more certain, so p10/p90 shift with p50 rather than narrowing
  toward it. Claiming otherwise would sell the user false precision.
- **An adjusted category's draw floors at zero.** A reduction larger than the modelled
  spend means "about nothing", never a projected inflow.
- **Overlapping changes on one category SUM.** Two windows on the same category add
  (−$200 from August, −$100 from November → −$300 from November), unlike `bill_amount` /
  `income_amount`, where the later-created window *replaces*. That is deliberate: these
  are deltas, not replacements, and "cut another $100" is the natural reading of a second
  change. It differs from its sibling kinds, so it is stated here and in the form's copy.

- **It inherits the Layer-2 activation gate, and the model's blind spots with it.** Below
  `LAYER2_MIN_HISTORY_MONTHS` (6 distinct months of categorized variable spend) there is
  no band to adjust. More subtly, the aggregate model is fitted from **liquid accounts
  only** — card-funded spend reaches the forecast through the card payment (ADR 0039 §2),
  not as a modelled category — so a household that puts all its dining on a rewards card
  has no dining band, and an override on it does nothing. Both follow from §8's
  data-progressive principle, but an inert control that silently accepts a change is a
  bad answer either way: the form says what is required, and narrowing the picker to the
  categories the model actually carries is tracked as follow-up work.

### 8. Data-progressive activation — the central principle (`6vj9`)

A **Forecast Readiness** score (0–100) is computed from data maturity: % of the
last 90 days categorized, recurring events with ≥ 3 actuals, backtest error within
envelope, and balance-observation freshness. The score **gates which layers'
output surfaces**:

- Below threshold → **only the deterministic line shows** (always trustworthy).
- As thresholds are crossed → confidence bands, scenario reliability, and spending
  insights **self-activate**. No new code ships at activation; capability is
  revealed.

The gate **is** the ADR-0018 non-advice enforcement: the product structurally
cannot present precision it hasn't earned. "X amount of data" is never an arbitrary
constant — it is this principled, multi-factor score.

### 9. Actualization loop (shared with the Money Inbox)

To compute readiness (≥ 3 actuals, backtest MAPE) and to calibrate models, the
system **matches forecast events ↔ realized transactions** (via the existing
`recurring_event_instances.linked_transaction_id` seam) and records actuals;
**backtest** replays past forecasts against realized values to score error. This is
the **same matching infrastructure** the Money Inbox and `personal-cfo-4k8d`
(bill paid-to-date) need — built once, shared.

### 10. Activation UX — communicate progressive capability

Because capabilities appear over time, the experience must **set the expectation up
front** and **announce each unlock** — otherwise early simplicity reads as "broken"
and later changes feel random:

- **Onboarding disclosure** — onboarding (the `n7bo`/`0v3v` flow) tells new users
  that the forecast starts as a simple deterministic projection and that **richer
  features (confidence ranges, scenarios, spending insights) appear automatically
  as they record more data** — "it grows with you."
- **Capability-unlock notification** — when readiness crosses a threshold and a
  capability activates, a **one-time, dismissible** notice (modal/toast) tells the
  user plainly, e.g. *"Your forecast now shows a likely range — you've recorded
  enough history for it to be meaningful,"* linking to the readiness factor that
  unlocked it. Shown once per capability (recorded, like the `audit_events`
  acknowledgement pattern).

### 11. Non-advice copy (ADR 0018)

All forecast copy is **descriptive, not prescriptive**; the readiness gate + band
rendering are the *mechanisms* that prevent implying false precision. ADR 0018 was
never written — this ADR establishes the non-advice **mechanism**; a dedicated
copy-language ADR 0018 should still be written for the wording rules + the
copy-review CI.

### 12. Per-account and per-group projection (`l8oh`) — addendum 2026-06-23

The Future Cash forecast computes, in addition to the single aggregate line, a
**running balance per liquid account and per cash group** over the horizon — the
foundation the multi-series chart (`l916`) and spreadsheet table (`ygjs`) consume.

**Computed by re-running the pure engine per series, not by changing it.** The
Layer-1 fold (`forecast_layer1`) is **linear**: a series' closing balance on day
*d* is its starting balance plus the cumulative sum of its events through *d*. So
running the *unchanged* engine once per series and summing the results is identical,
day by day, to the aggregate fold over the union of events with the summed starting
balance. The per-series assembly lives in the db-worker adapter; the pure engine is
untouched (and the `forecast-engine` crate-purity boundary is preserved).

**Event attribution.** Each projected flow is assigned to one series:

- **Income** → its `deposit_account_id`, when that account is a liquid-cash account.
- **Recurring bill / loan payment** → its `autopay_account_id`, when liquid.
- **Manual future entries** → their optional `account_id` when the user chose a liquid
  account (personal-cfo-4d8.24.3), attributed to that account's series exactly like income/
  bills. A manual entry stays a **forecast assumption**, never a ledger transaction — the
  account only routes the *projected* flow (so it never double-counts once a real
  transaction posts).
- Everything else — a manual entry with no chosen account, and income or bills whose
  account is unset or **non-liquid** (e.g. a bill autopaid from a credit card) — goes to a
  single synthetic **"Unallocated cash"** series.

**Why an Unallocated series (ratified 2026-06-23).** It keeps the aggregate
**unchanged** — today's Future Cash numbers do not shift — while staying honest about
what is attributed: the alternative of silently piling un-attributable flows onto a
"primary" account would misstate that account's line, and dropping them would
regress the forecast (a manual entry would stop moving Future Cash). The unallocated
series starts at zero (it is a flow bucket, not an account balance).

**Groups = the ADR 0028 cash tiers.** A group's series is the sum of its member
accounts' series: **Spendable** (checking + cash + unclassified liquid), **Reserve**
(savings + money market). **Net** = Spendable + Reserve + Unallocated, which equals
the aggregate. At day 0 the unallocated series is zero, so Net = Spendable + Reserve
(the ADR 0028 static net cash); over the horizon Net diverges from Spendable+Reserve
by exactly the accumulated unallocated flows. Custom groups (`uy82`) are a later
override of this type-based default.

**Reconciliation invariant (the `l8oh` acceptance test):** for every day,
`Σ per-account-series.closing (including Unallocated) == aggregate.closing`. It holds
by the linearity above and is asserted directly. Single-currency like the rest of
the forecast (`4n3x`); a multi-currency rollup is deferred with `ipui`.

### 13. Forecast Readiness — R1 subset (`6vj9`) — addendum 2026-06-24

§8 defines Forecast Readiness as a four-factor data-maturity score. **Three of the
four factors depend on infrastructure that does not exist in R1** — % of the last
90 days categorized (categorization, R2), recurring events with ≥ 3 actuals
(actualization, R3), and backtest error within envelope (the backtest job, R3) —
and there are **no Layer-2 confidence bands to gate yet** (R1 ships only the
deterministic line). So R1 computes a **principled subset** of the score from the
data-maturity signals that *do* exist, and surfaces it as an **informational trust
indicator** rather than an activation gate. The gate (§8) switches on with Layer-2
output (tracked by `nxgx`); the score's *framework* is built now so the later
factors slot in by extending the weighted sum, not by reworking the surface.

**The R1 factors** (all read from canonical state; nothing persisted — derived on
demand like the cash-availability snapshot, ADR 0029):

- **Coverage** — does the household have the inputs a forecast needs? An **active
  liquid account is a hard gate**: with none there is no starting balance, so the
  score is **0**. Given an account, income present and recurring bills present each
  add coverage. Without income or bills the deterministic line is honest but flat
  (it projects nothing changing), so coverage is the dominant factor.
- **Balance freshness** — how recently were balances confirmed? Measured from the
  **least-recently-asserted** liquid account (the weakest link, ADR 0027 balance
  assertions), decaying linearly to zero past a freshness horizon. A forecast that
  starts from a months-old balance has earned less trust.
- **Explained ratio** — how much of the asserted balance is explained by recorded
  postings vs. left as the auto-reconciling unexplained plug (`ueg6`)? **Neutral
  (full marks) when no transactions are recorded**, so the asserted-balance-only
  manual workflow (the *primary* path, ADR 0027) is never penalized; it dings only
  once recorded activity leaves a large residual relative to the balance.

**Score** = `round(100 × (0.5·coverage + 0.3·freshness + 0.2·explained))`, forced to
0 with no liquid account. Weights and the freshness horizon are **calibration
constants** named in the db-worker computation, tunable without an interface change;
the dashboard always shows the **per-factor breakdown** so the number is transparent
and each factor links to the action that improves it (add income, update a balance,
record transactions). Computed in db-worker (`forecast_readiness`) and exposed over
IPC — the right home as the §8 factors arrive, since those are all backend reads.

**Deferred to `nxgx`:** the three R2/R3 factors and the activation gate (hiding
Layer-2 bands / scenario reliability / insights below threshold — the ADR 0018
non-advice enforcement), which needs Layer-2 probabilistic output to exist before
there is anything to gate.

### 13a. Readiness factors + Layer-2 gating after calibration — addendum 2026-06-28 (`nxgx`)

With Layer-2 probabilistic output now shipped (the spend model `9h1s`, calibrated to
~80% coverage in `5ie.2`), the deferred half of §8 — additional factors plus the
activation gate — partially lands. Two refinements to §8:

**1. Calibrated bands change the gate's job.** §8 framed the gate as preventing *false
precision*: hide the band until enough data makes it trustworthy. After `5ie.2` the band
is **honest at any depth** — its P10/P90 is a Student-t prediction interval, *wide* when
history is thin and narrowing as evidence accrues. A thin-history band is therefore not
misleading, just honestly uncertain. So the gate's role shifts from "hide misleading
precision" to **reveal the band once there is enough history for a meaningful (seasonal)
model, and let band width + the readiness factors communicate confidence.** The
minimum-history floor (≥ 6 months of categorized variable spend) remains — below it there
is no seasonal signal and the band is uninformatively wide — but it is a *usefulness*
threshold, not a *trust* threshold.

**2. Capabilities gate on their *relevant* factors, not the blended score.** §8 said "the
score gates which layers surface." Refined: the blended score is the overall **trust
indicator**, but each capability activates on the factor(s) that actually bear on it. The
Layer-2 band activates on **spending-data maturity** (categorized variable-spend history),
*not* on balance freshness or coverage — a household with stale balances but rich spending
history has earned the band. Gating a spend band on balance freshness would be incorrect.

**Factors added now** (both derived on demand from canonical state, like the R1 three):
- **Spending detail** (categorization) — share of the last 90 days of spend that is
  categorized. Uncategorized spend can't be modeled, so this is the band's precondition.
- **Spending history** — distinct months of categorized variable-spend history toward the
  Layer-2 threshold; this **is** the band's activation predicate, surfaced so the card
  explains what unlocks the range.

**Re-pinned weights** (db-worker calibration constants, tunable without an interface
change): coverage 0.30, freshness 0.20, explained 0.10, categorization 0.20,
spending-history 0.20.

#### Re-pin 2026-06-29 (`nxgx` slice 2) — actuals-backed recurrence

With actualization shipped (`46jq`, §17), the **recurrence-actuals** factor (`Verified
accuracy`) joins the score: per active recurring event (income + bills), up to 3 realized
actuals (distinct `exact`/`matched` realized dates in `forecast_actuals`) are earned; the
factor is the captured fraction, **neutral (1.0) when there are no recurring events** so a
balances-only setup is never penalized. It reads whatever actuals exist — actualization is
run daily-on-open (alongside the §15 persist), not on the readiness read.

**Re-pinned weights** (now six factors, sum 1.0): coverage 0.30, freshness **0.15**,
explained 0.10, categorization **0.15**, spending-history **0.15**, recurrence-actuals
**0.15**.

#### Re-pin 2026-06-29 (`nxgx` slice 3) — per-vault backtest MAPE

The seventh and final factor, **backtest-MAPE** (`Forecast accuracy`, §18), joins: the
latest recorded per-vault MAPE maps to a score (≤ 10% error → full credit, ≥ 30% → none,
linear), **neutral (1.0) until there's enough realized history to judge**. This is the
*per-vault* backtest (the household's own runs vs its actuals), distinct from the CI
fixture-vault backtest suite (`7fxd`), which stays blocked on the encrypted-fixture-vault
generator (`9ujs`).

**Re-pinned weights** (seven factors, sum 1.0): coverage **0.25**, freshness 0.15,
explained 0.10, categorization 0.15, spending-history 0.15, recurrence-actuals **0.10**,
backtest-mape **0.10**. This completes §8's factor model; `nxgx` closes.

#### Addendum 2026-07-13 — spending-history indicator counts card spend (interim, ADR 0050)

The **spending-history** factor above is described as "the band's activation predicate,"
shared with the Layer-2 gate so the two never disagree. That premise is refined for the
interim: the readiness *indicator* now counts categorized variable spend across **all**
accounts (liquid **and** credit cards), because a card-based household — whose discretionary
spend sits on credit cards — otherwise reads spending-history 0 despite hundreds of
categorized transactions (`personal-cfo-4d8.27.1.2`). The Layer-2 band's *aggregation* stays
liquid-only (card spend reaches cash via the card payment, ADR 0039 §2), so the indicator and
the band's activation gate **diverge for card-heavy vaults** until the band re-model (ADR
0050) injects card variance at the card payment date and the two re-converge on a
card-inclusive model. This is deliberate interim honesty, not a permanent split.

Because the *indicator* now diverges from the band, the `forecast_band` capability-unlock
notice and the "projected spending range is active" detail gate on the band's **actual**
liquid-only activation predicate (`band_is_active`), not on the card-inclusive indicator score
— so the app never announces a range it does not draw. The indicator is also a raw
distinct-month count that, unlike the band's history, does not apply the ordinary/extraordinary
classifier (ADR 0038) — a second, minor divergence axis, acceptable for a maturity signal now
that the capability gates on the true band predicate.

#### Addendum 2026-07-13 — realized cash-flow history: derive on read, never fabricate (cf-history)

The Cash Flow view (ADR 0050) shows realized balance **history** behind the forward cone. History
is not stored — it is derived on read by folding each liquid account's balance backward from
today's assertion-anchored balance over the ledger postings: `bal(D) = bal_today − Σ(postings
dated after D)` (`compute_cash_flow_history`, `personal-cfo-4d8.27.5.2`). The **honesty rule**: a
series is clamped to the account's earliest real data (its first ledger posting), so nothing from
before the account existed is fabricated — a young household simply sees a shorter history, never
invented balances. An account with no postings at all shows only today's (asserted) balance. This
is the backward-looking analog of the forward cone's tight-near-term / honest-width principle, and
it satisfies the history-honesty half of `personal-cfo-4d8.27.5.1` (the data-completeness *gating*
half — unlocking the view once there is enough history — is `personal-cfo-4d8.27.5.5`).

**Gating half (2026-07-20, `personal-cfo-4d8.27.5.5`):** the `cash_flow_history` capability
self-activates once the earliest **liquid** posting is ≥ 30 days before household-local today —
the same `MIN(posting_date)` anchor the honesty clamp folds back to, derived on read. It fires
the one-time capability-ladder notice (§10's fires-once rule) and nothing more: the history view
renders honestly at any depth, so the unlock is an **announcement, not a hard gate**. The
onboarding wizard's Accounts step teaches the same ("30 days of recorded history shows where your
money has actually been").

### 18. Per-vault backtest — forecast accuracy from the household's own history (`nxgx`) — addendum 2026-06-29

`crates/db-worker/src/forecast_backtest.rs` scores how accurate the vault's *own* past
forecasts have been. It needs no synthetic fixtures: the daily-on-open persistence (§15) +
actualization (§17) already produce predicted↔realized pairs in `forecast_actuals`. It
aggregates them into a **MAPE** and appends a `forecast_backtest_results` row (the table's
first writer), run daily-on-open right after actualization.

- **Samples:** every *resolved* prediction — `exact`/`matched` (a real amount landed) and
  `missed` (predicted-but-absent, scored as a 100% error). Including misses is essential:
  matched rows are within the ±5% link tolerance by construction, so an amount-only MAPE
  could never leave the envelope — the predicted-but-absent events are what move it.
  `superseded` rows are excluded. A minimum of 3 samples before a MAPE is recorded.
- **Idempotent** per day (deterministic v5 id over `(model, day)`, `INSERT OR REPLACE`); the
  readiness factor reads the latest row only.
- **v1 gaps:** a single aggregate across the daily 365-day horizon (not yet bucketed by lead
  time); per-category MAPE and the CI fixture-vault suite remain `7fxd`/future work.

## The boundary: built now vs self-activating vs evolves-later

- **Built now (full architecture, deterministic-driven):** persisted reproducible
  pipeline (§3); assumption-event model + dependency edges + dirty ranges (§4);
  scenarios (§5); per-row explanation (§6); model registry + **L2–L4 baseline
  models** developed on synthetic data (§7); actualization/matching loop (§9);
  readiness score + gate (§8); the rich chart — axes, event markers, hover-to-
  explain, horizon picker (30/60/90/180/365d), minimum-cash-floor line, scenario
  selector, band rendering (§1); onboarding disclosure + unlock notifications (§10).
- **Self-activating (no new code; gated on data):** confidence bands, scenario
  reliability, spending insights — surface as readiness crosses thresholds.
- **Evolves later (via the registry; no pipeline rework):** better L2/L3/L4 models
  tuned to real data — **each gets its own follow-up ADR (0027+)** when designed
  *with* data. Advanced L3 (change-point) and L4 (full Monte Carlo) models land
  here too; their *harnesses* are built now, their *models* mature.

## Consequences

- **(+)** Complete, self-activating, honest system; **no "retrofit the engine" PR**;
  useful at every maturity stage (deterministic line now → bands/scenarios as data
  matures).
- **(+)** Reproducible + backtestable from day one; explainable; scenario-capable
  immediately on deterministic data.
- **(+)** The first chart shipped *is* the full chart (collapsed band) — built once.
- **(−)** The **largest arc** we have taken on; the assumption-event + registry +
  actualization machinery is real, irreducible complexity.
- **(−)** L2–L4 baselines are validated only on **synthetic** data until real data
  accrues → **synthetic-data fidelity (`9ujs`) is load-bearing**.
- **(−)** Pulls in **transaction-matching/actualization** as a hard dependency
  (shared with the Money Inbox / `4k8d`).
- **(−)** Persisting runs adds storage + a **retention policy** decision (how many
  runs/snapshots to keep for backtest) — see open question.
- Relates to ADR 0003 (typed explanation), 0009 (persisted forecast outputs are a
  new materialized surface), 0011 (assumption events are immutable), 0018 (non-
  advice mechanism), 0021 (calendar/tz).

## Alternatives considered

- **Primitive deterministic chart now, retrofit bands/scenarios later.** Rejected:
  the chart, IPC DTO, and storage would be rebuilt for the band shape — exactly the
  rework the owner wants to avoid.
- **Defer all of L2–L4 to a future phase.** Rejected: perpetual "later," integration
  debt, and a big-bang engine PR that never lands cleanly.
- **Build full *tuned* L2–L4 models now, ungated.** Rejected twice over: no data to
  tune against → wrong models = rework; and showing bands before they are
  trustworthy violates non-advice.
- **Build L2–L4 now but gate output on data readiness (this ADR),** with a model
  registry + baselines + synthetic-driven development. Accepted: neither speculative
  nor a shortcut.

## Decisions ratified (2026-06-21)

Decisions 1–7 were ratified as proposed; the two open questions are resolved (8–9).

1. **One band-shaped row** (P10/P50/P90 + provenance) across all layers; deterministic = collapsed band.
2. **Reproducible persisted pipeline** replaces on-demand compute (input snapshot + model registry).
3. **Assumption-event model** as the input/scenario/explanation/invalidation backbone.
4. **Pluggable models behind a registry**, shipping defensible baselines, developed on synthetic data.
5. **Data-progressive activation** via the Readiness gate as the central principle (the non-advice mechanism).
6. **Actualization/matching** built now as a hard dependency (shared with the Money Inbox).
7. **Activation UX**: onboarding disclosure + one-time capability-unlock notifications.
8. **Forecast-run retention (resolved):** keep the latest N runs + periodic snapshots for backtest; prune the rest. Finalized in the persistence bead (`eqfw`).
9. **ADR 0018 non-advice copy (resolved):** written alongside the chart (`eqzs`/`w5dn`), where the copy lands; tracked by `9xq`.

## Bead plan (ratified — the "just build" backlog)

**Phase F1 — reproducible spine**
- `5u2` assumption_events + dependency_edges + dirty_ranges (input/scenario/explain backbone)
- `0mg` forecast-state schema: actuals + quality_scores + backtest_results + risk_flags + `scenarios` (named definitions only). Scenario overlays are scenario-scoped assumption events (§5) — **no** `scenario_events` table; `model_registry` is owned by `eqfw`.
- `eqfw` input snapshots + forecast-run persistence + reproducibility metadata (replaces on-demand compute; owns the `model_registry` table)
- `tu2i` forecast row **band shape** (P10/P50/P90) + provenance — the §1 keystone *(NEW)*
- `ltj9` dirty-range invalidation worker (interactive tier)

**Phase F2 — deterministic-full UX (useful immediately)**
- `eqzs` Future Cash chart — built for bands; renders a collapsed line until L2 activates
- `vkge` per-row explanation · `q6gh` manual future entries · `xd8m` assumption supersession/history
- `6zep` scenario overlays + scenario events
- `uipt` First Forecast Wizard **+ progressive-activation disclosure**
- `w5dn` percentile copy review + `9xq` (ADR 0018 non-advice)

**Phase F3 — layers + the activation gate**
- `46jq` forecast actualization + transaction matching (shared w/ Money Inbox / `4k8d`)
- `6vj9` Forecast Readiness — **the data-progressive activation gate** (§8)
- `egon` capability-unlock notification — the activation modal *(NEW)*
- `9h1s` Layer-2 statistical baseline (empirical quantiles; synthetic-validated via `9ujs`)
- `6i2k` Layer-3 behavioral harness · `e596` Layer-4 Monte Carlo harness
- `7fxd` backtest suite · `fqbm` cash-availability / floor model

**Phase F4 — maturation (own ADRs 0027+)**
- tuned `9h1s` / `6i2k` / `e596` models (designed *with* real data) · `uodn` ML-stack discipline · `6rhn` backtest CLI

Phase association: F1–F2 are tracked by the R1 gate epic `hlh` (Phase 1); F3–F4 extend
into R2/R3 (`1xz3` / `jt21`). The deterministic line is useful from the end of F2;
bands/scenarios self-activate through F3 as readiness rises.

### 14. Recurring transfers in the per-account forecast (`npoe`) — addendum 2026-06-27

A **transfer** moves cash between two of the user's own accounts (ADR 0007 double-entry;
no system counter-account). A **recurring** transfer is a scheduled definition
(`recurring_transfers`: source + destination liquid accounts, amount, frequency, anchor)
that projects on the same pay-schedule machinery as bills and income.

**Attribution (extends §12).** Each projected occurrence emits **two legs**: `−amount`
attributed to the **source** account's series and `+amount` to the **destination**
account's series, both with `EventKind::Transfer` (same-day priority 2: after income and
bills, before manual entries). Unlike income/bills — attributed to *one* account via the
entity→account lookup (`build_attribution`) — a transfer names *both* accounts directly,
so `compute_by_account` injects its legs straight into the source and destination
partitions rather than through that lookup.

**Aggregate is unchanged.** A liquid↔liquid transfer is net-zero for total liquid cash,
so the **aggregate** forecast (`compute`) omits transfers entirely — including both legs
would only add `−amount + amount = 0` on the day. The **reconciliation invariant** (§12,
`Σ per-account closings == aggregate`) still holds: the two legs sum to zero across the
per-account series, matching the aggregate that omitted them.

**Scope.** v1 is liquid-cash ↔ liquid-cash (both legs land in the liquid forecast).
Mixed-role transfers (e.g. a credit-card payment, asset→liability) are deferred — the
liability leg needs normal-balance sign handling and would not appear in the liquid-only
forecast. A future multi-currency transfer is gated on the same FX work as the rest of
the forecast (`ipui`).

> **Resolved by ADR 0035 (2026-06-30, `6wk.1`).** The mixed-role deferral above is lifted:
> for an **asymmetric** transfer (liquid source, non-liquid destination — a debt payment or
> an investment contribution) the liquid leg is a real outflow that **does** count in the
> aggregate liquid forecast (it does not net to zero), and the non-liquid leg lives in the
> destination account's own projection. The §12 reconciliation invariant is restated there to
> sum over the liquid accounts only. See ADR 0035 §3.

### 15. Activating the persisted pipeline — daily-on-open cadence (`5ie.3`) — addendum 2026-06-29

§3 made a forecast a **persisted run** and `eqfw` built the machinery
(`generate_and_persist_forecast`: capture an input snapshot, dedup it by content hash,
write `forecast_runs` + `forecast_rows`). But the machinery shipped **dormant** — no
production code path ever called it; only tests did. The dashboard still reads the
on-demand `future_cash_forecast`. This addendum decides **when a run persists**, which
§3 left open, because the actualization loop (§9, `46jq`) cannot score anything until a
*history* of past runs exists to compare against later reality.

**Decision: persist one run per day, on vault open** (user-confirmed 2026-06-29).

- **Trigger.** After a successful `unlock_vault`, the app calls the kernel persistence
  path once. It is best-effort: a persistence failure is logged and **does not block
  unlock** (the forecast history is a derived convenience, not a correctness invariant of
  opening the vault). It runs at the open seam, **not** in a query handler — reads never
  write.
- **Dedup key = `(input content_hash, calendar day)`.** `capture_snapshot` already dedups
  the *snapshot* row by content hash, but `persist_run_and_rows` always wrote a *new*
  run. The activation path adds a run-level guard: if a `forecast_run` already exists
  whose `assumptions_hash` matches today's input content hash **and** whose
  `generated_at` falls on the current calendar day, the call is a no-op (`Ok(None)`).
  So a stable household that opens the app five times in a day persists exactly one run;
  changing a forecast input (new content hash) persists a fresh run the same day; a new
  day with unchanged inputs persists again — yielding the daily time series actualization
  needs.
- **Horizon.** The persisted daily run uses the **maximum** user-facing horizon (365d,
  the 1Y option) rather than the dashboard default (90d): the snapshot is the forward
  *record* actualization scores against, so capturing the fullest horizon lets later
  scoring choose any sub-window. Horizon does not enter the dedup key (it is not part of
  the input content hash).
- **Retention** is unchanged from the resolved-point 8 plan (keep latest N + periodic
  snapshots, prune the rest); pruning is a follow-on, not part of activation.

**Why daily-on-open and not change-driven or manual.** Change-driven persistence captures
every input edit (good for run-to-run diffs) but does not guarantee the regular,
time-spanning cadence actualization scoring wants, and is burstier; manual snapshots leave
coverage sparse and depend on the user. Daily-on-open is the cheapest trigger that
satisfies the actualization correctness constraint — runs that exist at points in time
across the horizon — and dedup keeps it idempotent within a day.

### 16. Building the actualization matching seam — recurring instances (`5ie.4`/`5ie.5`) — addendum 2026-06-29

§9 described the actualization loop as matching forecast events ↔ realized transactions
"via the existing `recurring_event_instances.linked_transaction_id` seam." That phrasing
was **aspirational**: the table was migrated but had **zero writers** — nothing projected
schedule occurrences or linked transactions — and `ledger_transactions` carry no
attribution to a forecast source entity. This addendum **builds** that seam, as the
foundation `46jq` (actualization) consumes.

**Recurring instances are a derived read model** (`crates/db-worker/src/recurring_instances.rs`),
in the `money_inbox` / `transaction_display` mold: full rebuild, idempotent, deterministic.

- **Projection (`5ie.4`).** For each active income source + in-forecast recurring
  obligation, expand occurrences over `[window_start, today + 365d]` via the **same**
  `PaySchedule::pay_dates` the forecast uses — through **shared schedule readers**
  (`crates/db-worker/src/schedule_sources.rs`) that both the forecast's
  `collect_income_events`/`collect_bill_events` and this projection call. Sharing the
  source fetch is what guarantees an instance and its forecast row carry the same
  `(entity, date, amount)`; a divergence (e.g. a `WHERE` added to one) can't happen
  silently. One `recurring_event_instances` row per occurrence, `status='scheduled'`.
  `window_start` = the earliest realized posting (so already-due occurrences are
  linkable), bounded to a 12-month fallback. The surrogate id is a **v5 hash of
  `(entity, scheduled_date)`** so a rebuild is byte-identical. `recurring_event_id`
  stores the forecast source entity id — an `income_sources.id` or a
  `recurring_events.id` — unified exactly as the forecast unifies them under
  `source_event_id`.
- **Linking (`5ie.5`).** After projecting, link each scheduled instance to the realized
  **liquid-account** ledger posting that satisfies it (`status='paid'` +
  `linked_transaction_id`): same sign (income inflow / obligation outflow),
  `scheduled_date ± 7d`, magnitude within `max(5%, 500 minor)` of expected. Deterministic
  greedy assignment — each posting links ≤1 instance and each instance ≤1 posting;
  contention resolves nearest-date → nearest-amount → `(transaction_id, index)`. Unmatched
  instances stay `scheduled`. Linking is part of the **projection rebuild**, not a
  command-path mutation, so the sensitive record/import write path is untouched. This
  durable, inspectable per-occurrence link is the precise seam (vs. a transient
  amount/date match computed at scoring time).

**Scope.** v1 expands the **base** schedule only (no scenario overlay / per-window
overrides); actualization scores the base deterministic run. The projection is rebuilt
**on demand by its consumer** (`46jq`), not wired into every write — there is no live UI
consumer yet, so per-command refresh would be premature coupling. Precision upgrades
(merchant-aware linking once Arc B's entity layer lands; per-occurrence overrides) are
later work.

### 17. Actualization scoring — forecast rows vs linked instances (`46jq`) — addendum 2026-06-29

With the instance seam built (§16), actualization (`crates/db-worker/src/forecast_actualize.rs`)
scores each persisted forecast row against reality and writes `forecast_actuals`. It is
a **full recompute**, idempotent (deterministic v5 id over `(run, row)`; the
`(forecast_run_id, forecast_row_id)` dedup key is honored), rebuilt on demand by
`actualize_forecasts` (which refreshes the instances first).

For every forecast row of an actualizable `source_type` (`income` / `recurring_bill` /
`loan_payment`) dated **on or before today**, joined to its instance by
`(source_id = recurring_event_id, date = scheduled_date)`:

- **superseded** — a strictly-newer run also predicted this `(entity, date)`. Only the
  newest run's prediction is scored; stale duplicates are recorded as `superseded` so
  quality scoring doesn't double-count. Checked first.
- **exact** — the instance is `paid` (linked) and the realized posting's amount is within
  **1%** (floor 100 minor) and date within **2 days** of the prediction.
- **matched** — `paid`, but the realized amount or date drifts beyond the tight band
  (still within the looser ±5% / ±7d *linking* tolerance, by construction).
- **missed** — a scheduled (unlinked) occurrence whose date is now past; `realized_amount
  = 0`, no `matched_transaction_id`.

The realized amount/date come from the linked transaction's **liquid-cash posting**;
currency from the instance. Rows whose entity/schedule has since changed (no current
instance) are **not** scored in v1 — a documented gap, acceptable because the persisted
run's prediction is only meaningful against a still-existing schedule. This feeds the
actuals-backed readiness factors + the accuracy surface (A3, `nxgx`).

### 19. Early-confirm / actualize-forward — satisfying a projected obligation ahead of its date (`5ie.7`) — addendum 2026-06-30

A user often settles a future obligation **early**: a card payment due the 20th gets paid on
the 16th, and both the card and the paying account should reflect it **from today**, not wait
for the 20th. This forces a choice without breaking the derived-read-model invariant —
`recurring_event_instances.status` is **derived** from canonical state (§16, `5ie.4`/`5ie.5`),
never user-writable. Decision:

- **(a) Early-confirm posts a real, balanced ledger transaction** — a Transfer (debit the
  paying liquid account, credit the liability) for a `credit_card_payment` / `loan_payment`
  obligation, or a `RecordTransaction` outflow for a plain bill — composing the existing
  kernel commands. It does **not** mutate instance status directly; status stays derived.
- **(b) The posting links to the targeted instance and suppresses its future projection.** On
  the next instance-projection rebuild, the posted transaction **links** to the intended
  occurrence (`status = paid`, `linked_transaction_id` set) **even though `actual_date`
  precedes `scheduled_date`** — the user explicitly asserted this pairing, so the link
  succeeds regardless of the `5ie.5` automatic-matching tolerance window (which governs only
  *unattributed* heuristic matching). The still-future scheduled occurrence is then treated as
  **fulfilled** and is **not projected again**, so the forecast shows exactly one outflow
  across the early-pay→due-date span (no double-count). This closes the documented §17 gap:
  `superseded` is *run-vs-run* dedup, a different concern from *a future occurrence satisfied
  early* — the latter is handled here by link-and-suppress.
- **(c) Idempotent on `(recurring_event_id, scheduled_date)`.** Confirming the same occurrence
  twice is a no-op; un-confirming reverses cleanly (voids the posting, the occurrence projects
  again).
- **(d) A neutral control, not a nudge (ADR 0018).** The affordance is *"mark this paid"* with
  an editable date/amount — never *"pay early to avoid a shortfall."* It is symmetric with the
  early-paycheck case on the inflow side.

This is the kernel mechanism behind the `5ie.9` `ConfirmObligationEarly` feature; the Money
Inbox "upcoming obligation" item (`92x7`) offers it as a resolution action.

## Addendum (2026-07-08, personal-cfo-4d8.24.4): drift-tolerant recurring-instance matching

Owner dogfooding: a bill scheduled for the 15th posts on the 13th/17th (holidays, weekends, bank
closures), and a variable bill's amount wanders. §9's instance-linking matched a realized posting to
a scheduled occurrence on **sign + date-window + amount-band** only, so a drifted amount (beyond the
5%/$5 band) failed to link even when the payee was obviously the same merchant. This adds a **payee
signal**.

**The match rule (recurring_instances.rs `assign_links`).** A realized liquid-cash posting links to a
scheduled occurrence when **all** of:

1. **Sign matches** the schedule kind (income → inflow, obligation → outflow). Unchanged, non-negotiable.
2. **Date within the window** `±RECURRING_MATCH_WINDOW_DAYS` (7 days) of the scheduled date — covers
   weekend/holiday drift. This is now a **single shared constant** used by both the instance matcher and
   the forecast's confirmed-obligation suppression (§9), so the two windows can never diverge.
3. **Amount within the band OR the payee matches.** The amount band (max 5%, floor $5) is unchanged; the
   payee is the normalized merchant key of the schedule's name vs the posting's `counterparty` (falling
   back to `memo`). A payee match **admits an out-of-band amount** (the drifted-amount case); a
   within-band amount **admits a missing/mismatched payee** (backward-compatible — postings without
   detail still link as before). A mismatched payee **and** an off-band amount does **not** link.

**Tie-break / ranking.** When an occurrence has several candidate postings (or vice-versa), the greedy
assignment ranks by, in order: **payee-match first**, then nearest date, then nearest amount, then
`(transaction_id, occurrence index)`. Payee-first ranking is what keeps two same-sign, similar-amount
occurrences in one window (distinct merchants) each linking to *their own* posting instead of
cross-linking. Each posting links ≤1 occurrence and each occurrence ≤1 posting.

**Determinism preserved.** The v5 instance ids are `(entity, scheduled_date)` hashes — unchanged; a
rebuild over identical inputs is byte-identical. The payee signal only reorders/admits candidate links;
it introduces no clock or randomness.

**Rationale.** ±7 days covers the realistic settlement drift without spanning a monthly cadence
(30 days). The payee signal is the strongest same-merchant evidence when the amount is unreliable
(variable bills); requiring payee-OR-amount (not payee-AND-amount) avoids dropping the common case
where imported postings lack a clean counterparty. A future refinement could weight partial payee
similarity; v1 uses exact normalized-key equality (the same `normalize_merchant` recurring detection uses).

## Addendum (2026-09-02, personal-cfo-xtz5): auto-reconciling projections against real transactions

Owner dogfooding (2026-09-01): projected rows and manual future entries stayed
on the Cash Flow surface after the real transaction had already synced or been
imported. Reconciliation is now automatic, built on the seams this ADR already
shipped:

1. **Linked instances suppress projections — by exact date, for
   still-projected occurrences, bills AND income.** (Tightened by adversarial
   review: reusing the ±7-day nearest-match for linked dates silently ate
   NEXT week's occurrence for any weekly cadence with linked history, and a
   confirmed-plus-linked payment consumed two occurrences.) The rules:
   a linked instance suppresses only the occurrence with its **exact**
   scheduled date (instances and the forecast expand the same PaySchedule
   grid — drift tolerance stays confined to explicit confirms, whose stored
   date can drift after a due-date edit); only instances with
   `scheduled_date >= forecast start` participate (a past paid occurrence's
   money and projection are both already behind the starting balance); and a
   confirm covered by a linked date within the tolerance is skipped, so one
   real payment consumes exactly one projected occurrence. Income gets the
   identical treatment — an early-arriving synced paycheck no longer
   double-counts. The past-due confirm queue already excluded linked
   instances; the forecast now agrees with it.
2. **One-off manual entries get their own deterministic matcher.** A new
   rebuildable projection, `manual_entry_links` (assumption_event_id →
   linked_transaction_id + matched date), is rebuilt alongside the instance
   seam. Match rule, deliberately strict for v1: the entry must carry an
   `account_id`; the transaction must be committed, non-voided, on that
   account, with the **exact** entry amount, within ±7 days
   (`RECURRING_MATCH_WINDOW_DAYS`), not already linked to a recurring
   instance, and each transaction is claimed by at most one entry (greedy
   nearest-date, ties broken by ids — byte-stable rebuilds). Matched entries
   drop out of `collect_manual_events` (the real flow is in the balance) and
   surface as **matched** in the entries UI — visible, descriptive
   (ADR 0018), and reversible by editing the entry. Unattributed entries are
   never auto-matched; the UI says so.
3. **Corroboration is already covered.** A manual confirm followed by the
   bank's copy of the same payment is caught by the cross-source dedupe layer
   (ADR 0014 §3 addendum): the synced copy waits in the Money Inbox with the
   matched counterpart attached, and skipping records the corroboration
   decision durably. `linked_transaction_id` is the instance-side audit
   trail.
4. **Freshness bound.** The seam (and the new entry matcher) rebuild on every
   sync and import ingest — **best-effort**: the rows are committed, the
   projections are derived and idempotent, and the next queue read heals a
   miss, so a rebuild failure warns rather than failing a successful ingest —
   and on the reads that already rebuilt it (past-due queue, actualization).
   A forecast rendered between a manual ledger write and the next
   seam-refreshing touch may briefly project an already-recorded flow; the
   next ingest or queue read heals it.
